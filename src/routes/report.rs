use crate::handlers::auth::Claims;
use crate::models::all_models::ReportedType;
use actix_web::{web, HttpMessage, HttpRequest, HttpResponse, Responder};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

//Create Report Request
#[derive(Debug, Deserialize, Serialize)]
pub struct CreateReportRequest {
    pub reported_user_id: Uuid,
    pub reason: String,
    pub reported_type: ReportedType,
    pub reported_item_id: Uuid,
}

//Create Report
//Create Report Input: HttpRequest(JWT Token), CreateReportRequest
//Create Report Output: Uuid (report_id)
pub async fn create_report(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    payload: web::Json<CreateReportRequest>,
) -> impl Responder {
    // Ensure the request is authenticated
    if let Some(claims) = req.extensions().get::<Claims>() {
        let reporter_id = claims.id;

        // Validate input
        if payload.reason.trim().is_empty() {
            return HttpResponse::BadRequest().body("Reason cannot be empty");
        }

        // Verify reported user exists
        let user_exists =
            sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM users WHERE user_id = $1)")
                .bind(payload.reported_user_id)
                .fetch_one(pool.get_ref())
                .await;

        match user_exists {
            Ok(exists) => {
                if !exists {
                    return HttpResponse::BadRequest().body("Reported user does not exist");
                }
            }
            Err(e) => {
                eprintln!("Error checking user: {:?}", e);
                return HttpResponse::InternalServerError().body("Error validating reported user");
            }
        }

        // Verify reported item exists based on type
        let item_exists = match payload.reported_type {
            ReportedType::Message => {
                sqlx::query_scalar::<_, bool>(
                    "SELECT EXISTS(SELECT 1 FROM messages WHERE message_id = $1)",
                )
                .bind(payload.reported_item_id)
                .fetch_one(pool.get_ref())
                .await
            }
            ReportedType::GroupChatMessage => sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS(SELECT 1 FROM group_chat_messages WHERE group_chat_message_id = $1)",
            )
            .bind(payload.reported_item_id)
            .fetch_one(pool.get_ref())
            .await,
            ReportedType::GroupChat => {
                sqlx::query_scalar::<_, bool>(
                    "SELECT EXISTS(SELECT 1 FROM group_chats WHERE group_chat_id = $1)",
                )
                .bind(payload.reported_item_id)
                .fetch_one(pool.get_ref())
                .await
            }
            ReportedType::User => {
                sqlx::query_scalar::<_, bool>(
                    "SELECT EXISTS(SELECT 1 FROM users WHERE user_id = $1)",
                )
                .bind(payload.reported_item_id)
                .fetch_one(pool.get_ref())
                .await
            }
            ReportedType::Post => {
                sqlx::query_scalar::<_, bool>(
                    "SELECT EXISTS(SELECT 1 FROM posts WHERE post_id = $1)",
                )
                .bind(payload.reported_item_id)
                .fetch_one(pool.get_ref())
                .await
            }
            ReportedType::Comment => {
                sqlx::query_scalar::<_, bool>(
                    "SELECT EXISTS(SELECT 1 FROM comments WHERE comment_id = $1)",
                )
                .bind(payload.reported_item_id)
                .fetch_one(pool.get_ref())
                .await
            }
        };

        match item_exists {
            Ok(exists) => {
                if !exists {
                    return HttpResponse::BadRequest().body("Reported item does not exist");
                }
            }
            Err(e) => {
                eprintln!("Error checking reported item: {:?}", e);
                return HttpResponse::InternalServerError().body("Error validating reported item");
            }
        }

        let query = "
            INSERT INTO reports 
                (reporter_id, reported_user_id, reason, reported_type, reported_item_id, status, created_at)
            VALUES 
                ($1, $2, $3, $4, $5, 'pending', NOW())
            RETURNING report_id";

        let result = sqlx::query_scalar::<_, Uuid>(query)
            .bind(reporter_id)
            .bind(payload.reported_user_id)
            .bind(&payload.reason)
            .bind(&payload.reported_type)
            .bind(payload.reported_item_id)
            .fetch_one(pool.get_ref())
            .await;

        match result {
            Ok(report_id) => {
                let response = json!({
                    "report_id": report_id,
                    "reported_user_id": payload.reported_user_id,
                    "reported_type": format!("{:?}", payload.reported_type),
                    "reported_item_id": payload.reported_item_id,
                    "status": "Pending"
                });
                HttpResponse::Created().json(response)
            }
            Err(e) => {
                eprintln!("Database error: {:?}", e);
                HttpResponse::InternalServerError().body("Error creating report")
            }
        }
    } else {
        HttpResponse::Unauthorized().body("Authentication required")
    }
}

//Config Report Routes
// POST /reports/new
pub fn config_report_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(web::scope("/reports").route("/new", web::post().to(create_report)));
}
