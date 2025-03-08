use crate::handlers::auth::Claims;
use crate::models::all_models::UserRole;
use actix_web::{HttpMessage, HttpRequest, HttpResponse, Responder, web};
use chrono::{NaiveDate, NaiveDateTime};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

//User Info
#[derive(Serialize, Deserialize, sqlx::FromRow)]
struct UserInfo {
    pub user_id: Uuid,
    pub username: String,
    pub role: UserRole,
    pub avatar_url: String,
    pub created_at: NaiveDateTime,
    pub dob: NaiveDate,
    pub user_profile: String,
    pub bio: Option<String>,
    pub email_verified: bool,
    pub banned_until: Option<NaiveDateTime>,
    pub location: Option<Value>,
    pub interests: Option<Vec<String>>,
    pub experience: Option<Vec<String>>,
    pub available_days: Option<Vec<String>>,
    pub languages: Option<Vec<String>>,
    pub privacy: bool,
}
//Get Logged In User Info
//Get Logged In User Info Input: HttpRequest(JWT Token)
//Get Logged In User Info Output: UserInfo
pub async fn get_logged_in_user_info(pool: web::Data<PgPool>, req: HttpRequest) -> impl Responder {
    if let Some(claims) = req.extensions().get::<Claims>() {
        let user_id = claims.id;

        let query = sqlx::query_as::<_, UserInfo>(
            "SELECT user_id, username, role, avatar_url, created_at, dob, user_profile, bio, 
            email_verified, banned_until, location, interests, experience, available_days, languages, privacy
            FROM users WHERE user_id = $1"
        )
        .bind(user_id)
        .fetch_one(pool.get_ref())
        .await;

        match query {
            Ok(user) => HttpResponse::Ok().json(user),
            Err(_) => HttpResponse::InternalServerError().body("User not found"),
        }
    } else {
        HttpResponse::Unauthorized().body("Authentication required")
    }
}

//Public User Info
#[derive(Serialize, Deserialize, sqlx::FromRow)]
struct PublicUserInfo {
    username: String,
    role: String,
    avatar_url: String,
    user_profile: String,
    bio: Option<String>,
    interests: Option<Vec<String>>,
    experience: Option<Vec<String>>,
    languages: Option<Vec<String>>,
}

//Private User Info
#[derive(Serialize, Deserialize, sqlx::FromRow)]
struct PrivateUserInfo {
    username: String,
    role: String,
    avatar_url: String,
}

//User Privacy Check
#[derive(sqlx::FromRow,Serialize,Deserialize)]
struct UserPrivacyCheck {
    privacy: bool,
}

//Get User By Name
//Get User By Name Input: Path (/users/{username})
//Get User By Name Output: PublicUserInfo or PrivateUserInfo
async fn get_user_by_name(pool: web::Data<PgPool>, path: web::Path<String>) -> impl Responder {
    let username = path.into_inner();

    let privacy_result = sqlx::query_as::<_, UserPrivacyCheck>(
        "SELECT privacy FROM users WHERE username = $1"
    )
    .bind(&username)
    .fetch_one(pool.get_ref())
    .await;

    match privacy_result {
        Ok(privacy_data) => {
            if privacy_data.privacy {
                let private_user_result = sqlx::query_as::<_, PrivateUserInfo>(
                    "SELECT username, role, avatar_url FROM users WHERE username = $1"
                )
                .bind(&username)
                .fetch_one(pool.get_ref())
                .await;

                return match private_user_result {
                    Ok(private_user) => HttpResponse::Ok().json(private_user),
                    Err(_) => HttpResponse::NotFound().body("User not found"),
                };
            }

            let user_result = sqlx::query_as::<_, PublicUserInfo>(
                "SELECT username, role, avatar_url, user_profile, bio, interests, experience, languages
                FROM users WHERE username = $1"
            )
            .bind(&username)
            .fetch_one(pool.get_ref())
            .await;

            match user_result {
                Ok(user) => HttpResponse::Ok().json(user),
                Err(_) => HttpResponse::NotFound().body("User not found"),
            }
        }
        Err(_) => HttpResponse::NotFound().body("User not found"),
    }
}

//Update User Request
#[derive(Debug, Deserialize, Serialize)]
pub struct UpdateUserRequest {
    pub user_profile: Option<String>,
    pub bio: Option<String>,
    pub location: Option<Value>,
    pub interests: Option<Vec<String>>,
    pub experience: Option<Vec<String>>,
    pub available_days: Option<Vec<String>>,
    pub languages: Option<Vec<String>>,
    pub privacy: Option<bool>,
}
//Updated User Profile
#[derive(Serialize, sqlx::FromRow)]
pub struct UpdatedUserProfile {
    pub user_profile: String,
    pub bio: Option<String>,
    pub location: Option<Value>,
    pub interests: Option<Vec<String>>,
    pub experience: Option<Vec<String>>,
    pub available_days: Option<Vec<String>>,
    pub languages: Option<Vec<String>>,
    pub privacy: bool,
}
//Update User Profile
//Update User Profile Input: HttpRequest(JWT Token), UpdateUserRequest
//Update User Profile Output: UpdatedUserProfile
pub async fn update_user_profile(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    payload: web::Json<UpdateUserRequest>,
) -> impl Responder {
    if let Some(claims) = req.extensions().get::<Claims>() {
        let user_id = claims.id;

        let result = sqlx::query_as::<_, UpdatedUserProfile>(
            "UPDATE users 
            SET user_profile = COALESCE($1, user_profile),
                bio = COALESCE($2, bio),
                location = COALESCE($3, location),
                interests = COALESCE($4, interests),
                experience = COALESCE($5, experience),
                available_days = COALESCE($6, available_days),
                languages = COALESCE($7, languages),
                privacy = COALESCE($8, privacy)
            WHERE user_id = $9
            RETURNING user_profile, bio, location, interests, experience, available_days, languages, privacy"
        )
        .bind(payload.user_profile.as_ref())
        .bind(payload.bio.as_ref())
        .bind(payload.location.as_ref())
        .bind(payload.interests.as_ref())
        .bind(payload.experience.as_ref())
        .bind(payload.available_days.as_ref())
        .bind(payload.languages.as_ref())
        .bind(payload.privacy)
        .bind(user_id)
        .fetch_one(pool.get_ref())
        .await;

        match result {
            Ok(updated_user) => HttpResponse::Ok().json(updated_user),
            Err(e) => {
                eprintln!("Error updating user profile: {:?}", e);
                HttpResponse::InternalServerError().body("Failed to update profile")
            }
        }
    } else {
        HttpResponse::Unauthorized().body("Authentication required")
    }
}

pub async fn delete_user_account(pool: web::Data<PgPool>, req: HttpRequest) -> impl Responder {
    if let Some(claims) = req.extensions().get::<Claims>() {
        let user_id = claims.id;

        let result = sqlx::query("DELETE FROM users WHERE user_id = $1")
            .bind(user_id)
            .execute(pool.get_ref())
            .await;

        match result {
            Ok(_) => HttpResponse::Ok().body("Account deleted successfully"),
            Err(_) => HttpResponse::InternalServerError().body("Failed to delete account"),
        }
    } else {
        HttpResponse::Unauthorized().body("Authentication required")
    }
}

// Config User Data Routes
// GET /users/info
// GET /users/{username}
// PATCH /users/update-info
// DELETE /users/delete-user
pub fn config_user_data_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/users")
            .route("/info", web::get().to(get_logged_in_user_info))
            .route("/{username}", web::get().to(get_user_by_name))
            .route("/update-info", web::patch().to(update_user_profile))
            .route("/delete-user", web::delete().to(delete_user_account)),
    );
}

