use crate::handlers::auth::generate_jwt;
use crate::handlers::password::{hash_password, verify_password};
use crate::models::all_models::UserRole;
use actix_web::{HttpResponse, Responder, web};
use chrono::{NaiveDate, NaiveDateTime};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

//Create User Request
#[derive(Deserialize, Serialize)]
pub struct CreateUserRequest {
    pub username: String,
    pub email: String,
    pub password: String,
    pub dob: NaiveDate,
}

//Created User Response
#[derive(sqlx::FromRow, Serialize)]
pub struct CreatedUserResponse {
    pub user_id: Uuid,
    pub username: String,
    pub avatar_url: String,
}

//Create User
//Create User Input: CreateUserRequest
//Create User Output: CreatedUserResponse
pub async fn create_user(
    pool: web::Data<PgPool>,
    payload: web::Json<CreateUserRequest>,
) -> impl Responder {
    let avatar_url = format!(
        "https://ui-avatars.com/api/?name={}&background=random",
        payload.username
    );

    let password_hash = match hash_password(&payload.password) {
        Ok(hash) => hash,
        Err(_) => return HttpResponse::InternalServerError().body("Failed to hash password"),
    };

    let user_profile = "Nothing to see here...";

    let query = "INSERT INTO users (username, email, password_hash, dob, avatar_url, user_profile) \
                 VALUES ($1, $2, $3, $4, $5, $6) RETURNING user_id, username, avatar_url";

    let result = sqlx::query_as::<_, CreatedUserResponse>(query)
        .bind(&payload.username)
        .bind(&payload.email)
        .bind(password_hash)
        .bind(payload.dob)
        .bind(&avatar_url)
        .bind(user_profile)
        .fetch_one(pool.get_ref())
        .await;

    match result {
        Ok(record) => HttpResponse::Ok().json(record),
        Err(e) => {
            eprintln!("Database error: {:?}", e);
            HttpResponse::InternalServerError().json("Error creating user")
        }
    }
}

//Login Request
#[derive(Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

//User Auth
#[derive(sqlx::FromRow)]
struct UserAuth {
    pub user_id: Uuid,
    pub username: String,
    pub password_hash: String,
    pub avatar_url: String,
    pub role: UserRole,
    pub banned_until: Option<NaiveDateTime>,
}

//Login Response
#[derive(Serialize)]
pub struct LoginResponse {
    pub user_id: Uuid,
    pub username: String,
    pub avatar_url: String,
    pub token: String,
}

//Login
//Login Input: LoginRequest
//Login Output: LoginResponse
pub async fn login(pool: web::Data<PgPool>, payload: web::Json<LoginRequest>) -> impl Responder {
    // Query the user by username and fetch necessary fields
    let query = "
        SELECT user_id, username, password_hash, avatar_url, role, banned_until 
        FROM users WHERE username = $1";

    let user = sqlx::query_as::<_, UserAuth>(query)
        .bind(&payload.username)
        .fetch_one(pool.get_ref())
        .await;

    match user {
        Ok(user) => {
            // Check if the user is banned
            if let Some(banned_until) = user.banned_until {
                if banned_until > chrono::Utc::now().naive_utc() {
                    return HttpResponse::Forbidden().body("Your account is currently banned.");
                }
            }

            // Verify password
            let verified = match verify_password(&payload.password, &user.password_hash) {
                Ok(r) => r,
                Err(_) => {
                    return HttpResponse::InternalServerError().body("Error Verifying Password!");
                }
            };

            if verified {
                let token = match generate_jwt(user.user_id, user.username.clone(), user.role) {
                    Ok(t) => t,
                    Err(e) => {
                        eprintln!("Token generation error: {:?}", e);
                        return HttpResponse::InternalServerError().body("Token generation failed");
                    }
                };

                let response = LoginResponse {
                    user_id: user.user_id,
                    username: user.username,
                    avatar_url: user.avatar_url,
                    token,
                };

                HttpResponse::Ok().json(response)
            } else {
                HttpResponse::Unauthorized().body("Invalid credentials")
            }
        }
        Err(e) => {
            eprintln!("Error retrieving user: {:?}", e);
            HttpResponse::InternalServerError().body("Error logging in")
        }
    }
}

//Config User Auth Routes
// POST /auth/register
// POST /auth/login
pub fn config_user_auth_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/auth")
            .route("/register", web::post().to(create_user))
            .route("/login", web::post().to(login)),
    );
}
