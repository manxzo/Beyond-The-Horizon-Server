use crate::handlers::auth::Claims;
use crate::handlers::matching_algo::calculate_match_score;
use crate::handlers::ws::send_to_user;
use crate::models::all_models::{MatchUser, MatchingRequest, MatchingStatus};
use actix_web::{HttpMessage, HttpRequest, HttpResponse, Responder, web};
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

//Recommend Sponsors
//Recommend Sponsors Input: HttpRequest(JWT Token)
//Recommend Sponsors Output: Vec<MatchUser>
pub async fn recommend_sponsors(pool: web::Data<PgPool>, req: HttpRequest) -> impl Responder {
    if let Some(claims) = req.extensions().get::<Claims>() {
        let user_id = claims.id;

        let user_query = "
            SELECT user_id as id, dob, location, interests, experience, available_days, languages
            FROM users WHERE user_id = $1";

        let user_result = sqlx::query_as::<_, MatchUser>(user_query)
            .bind(user_id)
            .fetch_one(pool.get_ref())
            .await;

        if let Ok(member) = user_result {
            if member.location.is_none()
                || member.interests.is_none()
                || member.experience.is_none()
                || member.available_days.is_none()
                || member.languages.is_none()
            {
                return HttpResponse::BadRequest()
                    .body("Complete your profile before requesting a sponsor.");
            }

            let sponsor_query = "
                SELECT user_id as id, dob, location, interests, experience, available_days, languages
                FROM users WHERE role = 'sponsor'";

            let sponsors_result = sqlx::query_as::<_, MatchUser>(sponsor_query)
                .fetch_all(pool.get_ref())
                .await;

            match sponsors_result {
                Ok(sponsors) => {
                    let mut sponsor_scores: Vec<(MatchUser, f32)> = sponsors
                        .into_iter()
                        .map(|sponsor| {
                            let score = calculate_match_score(&member, &sponsor);
                            (sponsor, score)
                        })
                        .collect();

                    sponsor_scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

                    HttpResponse::Ok().json(sponsor_scores)
                }
                Err(e) => {
                    eprintln!("Failed to fetch sponsors: {:?}", e);
                    HttpResponse::InternalServerError().body("Failed to fetch sponsors.")
                }
            }
        } else {
            eprintln!("Failed to fetch user data: {:?}", user_result.err());
            HttpResponse::InternalServerError().body("Failed to fetch user data.")
        }
    } else {
        HttpResponse::Unauthorized().body("Authentication required")
    }
}

//Sponsor Request
#[derive(Debug, Deserialize, Serialize)]
pub struct SponsorRequest {
    pub sponsor_id: Uuid,
}

//Request Sponsor
//Request Sponsor Input: HttpRequest(JWT Token), SponsorRequest
//Request Sponsor Output: MatchingRequest
pub async fn request_sponsor(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    payload: web::Json<SponsorRequest>,
) -> impl Responder {
    if let Some(claims) = req.extensions().get::<Claims>() {
        let user_id = claims.id;

        // Check if there's already a pending request
        let check_query = "
            SELECT COUNT(*) FROM matching_requests 
            WHERE member_id = $1 AND sponsor_id = $2 AND status = 'pending'";

        let count: i64 = sqlx::query_scalar(check_query)
            .bind(user_id)
            .bind(payload.sponsor_id)
            .fetch_one(pool.get_ref())
            .await
            .unwrap_or(0);

        if count > 0 {
            return HttpResponse::Conflict().body("You have already requested this sponsor.");
        }

        // Ensure user has filled required fields before requesting
        let user_query = "
            SELECT location, interests, experience, available_days, languages
            FROM users WHERE user_id = $1";

        let user_result: Result<
            (
                Option<Value>,
                Option<Vec<String>>,
                Option<Vec<String>>,
                Option<Vec<String>>,
                Option<Vec<String>>,
            ),
            sqlx::Error,
        > = sqlx::query_as(user_query)
            .bind(user_id)
            .fetch_one(pool.get_ref())
            .await;

        match user_result {
            Ok((location, interests, experience, available_days, languages)) => {
                if location.is_none()
                    || interests.is_none()
                    || experience.is_none()
                    || available_days.is_none()
                    || languages.is_none()
                {
                    return HttpResponse::BadRequest()
                        .body("Complete your profile before requesting a sponsor.");
                }

                // Insert the matching request
                let insert_query = "
                    INSERT INTO matching_requests (member_id, sponsor_id, status, created_at)
                    VALUES ($1, $2, 'pending', NOW())
                    RETURNING matching_request_id, member_id, sponsor_id, status, created_at";

                let request_result = sqlx::query_as::<_, MatchingRequest>(insert_query)
                    .bind(user_id)
                    .bind(payload.sponsor_id)
                    .fetch_one(pool.get_ref())
                    .await;

                match request_result {
                    Ok(request) => {
                        // Send notification to sponsor via websocket
                        let message = serde_json::json!({
                            "type": "sponsor_request",
                            "message": format!("You have a new sponsorship request from user {}", claims.username)
                        });

                        send_to_user(&payload.sponsor_id, message).await;

                        HttpResponse::Ok().json(request)
                    }
                    Err(e) => {
                        eprintln!("Failed to request sponsor: {:?}", e);
                        HttpResponse::InternalServerError().body("Failed to request sponsor.")
                    }
                }
            }
            Err(e) => {
                eprintln!("Failed to fetch user data: {:?}", e);
                HttpResponse::InternalServerError().body("Failed to fetch user data.")
            }
        }
    } else {
        HttpResponse::Unauthorized().body("Authentication required")
    }
}

//Matching Request With User Info
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct MatchingRequestWithUserInfo {
    pub matching_request_id: Uuid,
    pub member_id: Uuid,
    pub sponsor_id: Uuid,
    pub status: String,
    pub created_at: NaiveDateTime,
    pub username: String,
    pub avatar_url: String,
}

//Check Matching Status
//Check Matching Status Input: HttpRequest(JWT Token)
//Check Matching Status Output: Vec<MatchingRequestWithUserInfo>
pub async fn check_matching_status(pool: web::Data<PgPool>, req: HttpRequest) -> impl Responder {
    if let Some(claims) = req.extensions().get::<Claims>() {
        let user_id = claims.id;

        // Get user role to determine which requests to show
        let role_query = "SELECT role FROM users WHERE user_id = $1";
        let role: Option<String> = sqlx::query_scalar(role_query)
            .bind(user_id)
            .fetch_one(pool.get_ref())
            .await
            .unwrap_or(None);

        let query = if role.as_deref() == Some("sponsor") {
            // For sponsors, show requests where they are the sponsor
            "
            SELECT mr.*, u.username, u.avatar_url
            FROM matching_requests mr
            JOIN users u ON mr.member_id = u.user_id
            WHERE mr.sponsor_id = $1
            ORDER BY mr.created_at DESC
            "
        } else {
            // For members, show requests they've made
            "
            SELECT mr.*, u.username, u.avatar_url
            FROM matching_requests mr
            JOIN users u ON mr.sponsor_id = u.user_id
            WHERE mr.member_id = $1
            ORDER BY mr.created_at DESC
            "
        };

        let result = sqlx::query_as::<_, MatchingRequestWithUserInfo>(query)
            .bind(user_id)
            .fetch_all(pool.get_ref())
            .await;

        match result {
            Ok(requests) => HttpResponse::Ok().json(requests),
            Err(e) => {
                eprintln!("Failed to fetch matching requests: {:?}", e);
                HttpResponse::InternalServerError().body("Failed to fetch matching requests.")
            }
        }
    } else {
        HttpResponse::Unauthorized().body("Authentication required")
    }
}

//Sponsor Response
#[derive(Debug, Deserialize, Serialize)]
pub struct SponsorResponse {
    pub matching_request_id: Uuid,
    pub accept: bool,
}

//Respond To Matching Request
//Respond To Matching Request Input: HttpRequest(JWT Token), SponsorResponse
//Respond To Matching Request Output: MatchingRequest
pub async fn respond_to_matching_request(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    payload: web::Json<SponsorResponse>,
) -> impl Responder {
    if let Some(claims) = req.extensions().get::<Claims>() {
        let sponsor_id = claims.id;

        // First, verify that this request is directed to this sponsor
        let verify_query = "
            SELECT mr.member_id, u.username 
            FROM matching_requests mr
            JOIN users u ON mr.member_id = u.user_id
            WHERE mr.matching_request_id = $1 AND mr.sponsor_id = $2
        ";

        let member_info: Option<(Uuid, String)> = sqlx::query_as(verify_query)
            .bind(payload.matching_request_id)
            .bind(sponsor_id)
            .fetch_optional(pool.get_ref())
            .await
            .unwrap_or(None);

        if let Some((member_id, _member_username)) = member_info {
            let update_query = "
                UPDATE matching_requests 
                SET status = $1 
                WHERE matching_request_id = $2 AND sponsor_id = $3
                RETURNING matching_request_id, member_id, sponsor_id, status, created_at";

            let new_status = if payload.accept {
                MatchingStatus::Accepted
            } else {
                MatchingStatus::Declined
            };

            let result = sqlx::query_as::<_, MatchingRequest>(update_query)
                .bind(new_status.to_string())
                .bind(&payload.matching_request_id)
                .bind(&sponsor_id)
                .fetch_one(pool.get_ref())
                .await;

            match result {
                Ok(updated_request) => {
                    // Send notification to member via websocket
                    let status_text = if payload.accept {
                        "accepted"
                    } else {
                        "declined"
                    };
                    let message = serde_json::json!({
                        "type": "sponsor_response",
                        "message": format!("Your sponsorship request has been {} by {}", status_text, claims.username)
                    });

                    send_to_user(&member_id, message).await;

                    HttpResponse::Ok().json(updated_request)
                }
                Err(e) => {
                    eprintln!("Failed to update matching request: {:?}", e);
                    HttpResponse::InternalServerError().body("Failed to update request.")
                }
            }
        } else {
            HttpResponse::BadRequest().body("This request is not directed to this sponsor.")
        }
    } else {
        HttpResponse::Unauthorized().body("Authentication required")
    }
}

//Config Matching Routes
// GET /matching/recommend-sponsors
// POST /matching/request-sponsor
// GET /matching/status
// PATCH /matching/respond
pub fn config_matching_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/matching")
            .route("/recommend-sponsors", web::get().to(recommend_sponsors))
            .route("/request-sponsor", web::post().to(request_sponsor))
            .route("/status", web::get().to(check_matching_status))
            .route("/respond", web::patch().to(respond_to_matching_request)),
    );
}
