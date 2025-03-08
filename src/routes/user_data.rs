use crate::handlers::auth::Claims;
use crate::handlers::b2_storage::B2Client;
use crate::models::all_models::UserRole;
use actix_multipart::Multipart;
use actix_web::{HttpMessage, HttpRequest, HttpResponse, Responder, web};
use chrono::{NaiveDate, NaiveDateTime};
use futures::{StreamExt, TryStreamExt};
use log::{error, info};
use mime_guess::from_path;
use sanitize_filename::sanitize;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{PgPool, Row};
use std::io::Write;
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
#[derive(sqlx::FromRow, Serialize, Deserialize)]
struct UserPrivacyCheck {
    privacy: bool,
}

//Get User By Name
//Get User By Name Input: Path (/users/{username})
//Get User By Name Output: PublicUserInfo or PrivateUserInfo
async fn get_user_by_name(pool: web::Data<PgPool>, path: web::Path<String>) -> impl Responder {
    let username = path.into_inner();

    let privacy_result =
        sqlx::query_as::<_, UserPrivacyCheck>("SELECT privacy FROM users WHERE username = $1")
            .bind(&username)
            .fetch_one(pool.get_ref())
            .await;

    match privacy_result {
        Ok(privacy_data) => {
            if privacy_data.privacy {
                let private_user_result = sqlx::query_as::<_, PrivateUserInfo>(
                    "SELECT username, role, avatar_url FROM users WHERE username = $1",
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

// Avatar upload response
#[derive(Serialize, Deserialize)]
pub struct AvatarUploadResponse {
    pub avatar_url: String,
}

// Upload avatar handler
pub async fn upload_avatar(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    mut payload: Multipart,
) -> impl Responder {
    // Extract user claims from request
    let ext = req.extensions();
    let claims = match ext.get::<Claims>() {
        Some(claims) => claims,
        None => return HttpResponse::Unauthorized().body("Unauthorized"),
    };

    // First, check if the user already has a custom avatar in B2
    let current_avatar_result = sqlx::query("SELECT avatar_url FROM users WHERE user_id = $1")
        .bind(claims.id)
        .fetch_one(pool.get_ref())
        .await;

    let current_avatar = match current_avatar_result {
        Ok(record) => record.get::<String, _>("avatar_url"),
        Err(e) => {
            error!("Error fetching current avatar URL: {:?}", e);
            return HttpResponse::InternalServerError().body("Failed to fetch current avatar");
        }
    };

    // Initialize B2 client
    let mut b2_client = match B2Client::new() {
        Ok(client) => client,
        Err(e) => {
            error!("Failed to initialize B2 client: {:?}", e);
            return HttpResponse::InternalServerError().body("Failed to initialize storage client");
        }
    };

    // If the current avatar is from B2 (not the default UI Avatars), delete it
    if current_avatar.contains("/file/") && !current_avatar.contains("ui-avatars.com") {
        // Extract filename from URL
        let filename = current_avatar.split('/').last().unwrap_or_default();

        // Delete file from B2
        if let Err(e) = b2_client.delete_file(filename).await {
            error!("Failed to delete old avatar from B2: {:?}", e);
            // Continue anyway to upload the new avatar
        } else {
            info!("Successfully deleted old avatar from B2");
        }
    }

    // Process the multipart form data
    let mut file_bytes: Option<Vec<u8>> = None;
    let mut file_name: Option<String> = None;
    let mut content_type: Option<String> = None;

    while let Ok(Some(mut field)) = payload.try_next().await {
        let content_disposition = match field.content_disposition() {
            Some(cd) => cd,
            None => continue,
        };

        if let Some(name) = content_disposition.get_name() {
            if name == "avatar" {
                // Get filename
                let original_filename = content_disposition
                    .get_filename()
                    .map(|f| sanitize(f))
                    .unwrap_or_else(|| format!("avatar_{}.jpg", Uuid::new_v4()));

                // Create a unique filename with user ID
                let extension = original_filename.split('.').last().unwrap_or("jpg");

                let unique_filename = format!("avatar_{}.{}", claims.id, extension);
                file_name = Some(unique_filename);

                // Guess content type from filename
                content_type = Some(
                    from_path(&original_filename)
                        .first_or_octet_stream()
                        .to_string(),
                );

                // Read file data
                let mut data = Vec::new();
                while let Some(chunk_result) = field.next().await {
                    match chunk_result {
                        Ok(chunk) => {
                            if let Err(e) = data.write_all(&chunk) {
                                error!("Error writing chunk to buffer: {:?}", e);
                                return HttpResponse::InternalServerError()
                                    .body("Error processing file upload");
                            }
                        }
                        Err(e) => {
                            error!("Error reading multipart chunk: {:?}", e);
                            return HttpResponse::InternalServerError()
                                .body("Error processing file upload");
                        }
                    }
                }

                // Check file size (limit to 5MB)
                if data.len() > 5 * 1024 * 1024 {
                    return HttpResponse::BadRequest().body("File too large (max 5MB)");
                }

                file_bytes = Some(data);
            }
        }
    }

    // Check if we have a file
    let (file_data, filename, mime_type) = match (file_bytes, file_name, content_type) {
        (Some(data), Some(name), Some(mime)) => (data, name, mime),
        _ => return HttpResponse::BadRequest().body("No avatar file provided"),
    };

    // Upload to B2
    let avatar_url = match b2_client
        .upload_file(&file_data, &filename, &mime_type)
        .await
    {
        Ok(url) => url,
        Err(e) => {
            error!("Failed to upload avatar to B2: {:?}", e);
            return HttpResponse::InternalServerError().body("Failed to upload avatar");
        }
    };

    // Update user's avatar URL in database
    let result =
        sqlx::query("UPDATE users SET avatar_url = $1 WHERE user_id = $2 RETURNING avatar_url")
            .bind(&avatar_url)
            .bind(claims.id)
            .fetch_one(pool.get_ref())
            .await;

    match result {
        Ok(record) => {
            let avatar_url: String = record.get("avatar_url");
            HttpResponse::Ok().json(AvatarUploadResponse { avatar_url })
        }
        Err(e) => {
            error!("Error updating avatar URL in database: {:?}", e);
            HttpResponse::InternalServerError().body("Failed to update avatar URL in database")
        }
    }
}

// Reset avatar handler
pub async fn reset_avatar(pool: web::Data<PgPool>, req: HttpRequest) -> impl Responder {
    // Extract user claims from request
    let ext = req.extensions();
    let claims = match ext.get::<Claims>() {
        Some(claims) => claims,
        None => return HttpResponse::Unauthorized().body("Unauthorized"),
    };

    // Get current avatar URL
    let current_avatar_result = sqlx::query("SELECT avatar_url FROM users WHERE user_id = $1")
        .bind(claims.id)
        .fetch_one(pool.get_ref())
        .await;

    let current_avatar = match current_avatar_result {
        Ok(record) => record.get::<String, _>("avatar_url"),
        Err(e) => {
            error!("Error fetching current avatar URL: {:?}", e);
            return HttpResponse::InternalServerError().body("Failed to fetch current avatar");
        }
    };

    // Check if the current avatar is from B2 (not the default UI Avatars)
    if current_avatar.contains("/file/") && !current_avatar.contains("ui-avatars.com") {
        // Initialize B2 client
        let mut b2_client = match B2Client::new() {
            Ok(client) => client,
            Err(e) => {
                error!("Failed to initialize B2 client: {:?}", e);
                return HttpResponse::InternalServerError()
                    .body("Failed to initialize storage client");
            }
        };

        // Extract filename from URL
        let filename = current_avatar.split('/').last().unwrap_or_default();

        // Delete file from B2
        if let Err(e) = b2_client.delete_file(filename).await {
            error!("Failed to delete avatar from B2: {:?}", e);
            // Continue anyway to update the database
        }
    }

    // Generate default avatar URL with UI Avatars
    let username = claims.username.clone();
    let default_avatar_url = format!(
        "https://ui-avatars.com/api/?name={}&background=random&size=256",
        username
    );

    // Update user's avatar URL in database
    let result =
        sqlx::query("UPDATE users SET avatar_url = $1 WHERE user_id = $2 RETURNING avatar_url")
            .bind(&default_avatar_url)
            .bind(claims.id)
            .fetch_one(pool.get_ref())
            .await;

    match result {
        Ok(record) => {
            let avatar_url: String = record.get("avatar_url");
            HttpResponse::Ok().json(AvatarUploadResponse { avatar_url })
        }
        Err(e) => {
            error!("Error resetting avatar URL in database: {:?}", e);
            HttpResponse::InternalServerError().body("Failed to reset avatar URL in database")
        }
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
            .route("/delete-user", web::delete().to(delete_user_account))
            .route("/avatar/upload", web::post().to(upload_avatar))
            .route("/avatar/reset", web::post().to(reset_avatar)),
    );
}
