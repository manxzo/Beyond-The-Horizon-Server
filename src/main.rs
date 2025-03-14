mod handlers;
mod middleware;
mod models;
mod routes;

use actix_cors::Cors;
use actix_identity::IdentityMiddleware;
use actix_session::{storage::CookieSessionStore, SessionMiddleware};
use actix_web::{
    cookie::{Cookie, Key, SameSite},
    middleware::Logger,
    web, HttpResponse,
};
use anyhow;
use chrono;
use handlers::ws::init_ws_routes;
use log::{error, info};
use middleware::{
    auth_middleware::AuthMiddleware, request_logger::RequestLogger,
    session_refresh_middleware::SessionRefreshMiddleware,
};
use routes::{
    admin::config_admin_routes, group_chats::config_group_chat_routes, posts::config_feed_routes,
    private_messaging::config_message_routes, report::config_report_routes,
    resources::config_resource_routes, sponsor_matching::config_matching_routes,
    sponsor_role::config_sponsor_routes, support_group_meetings::config_meeting_routes,
    support_groups::config_support_group_routes, user_auth::config_user_auth_routes,
    user_data::config_user_data_routes,
};
use serde_json;
use shuttle_actix_web::ShuttleActixWeb;
use shuttle_runtime::SecretStore;
use sqlx::PgPool;

#[shuttle_runtime::main]
async fn main(
    #[shuttle_runtime::Secrets] secrets: SecretStore,
) -> ShuttleActixWeb<impl FnOnce(&mut web::ServiceConfig) + Send + Clone + 'static> {
    // Configure and initialize logger safely

    // Log all available secrets with their values for testing
    let secret_names = vec![
        "SESSION_SECRET",
        "DATABASE_URL",
        "ALLOWED_ORIGINS",
        "RUST_LOG",
        "RUST_BACKTRACE",
        "B2_APPLICATION_KEY_ID",
        "B2_APPLICATION_KEY",
        "B2_BUCKET_ID",
        "UPLOAD_DIR",
    ];

    for name in &secret_names {
        if let Some(value) = secrets.get(name) {
            info!("Secret: {} = {}", name, value);
        } else {
            info!("Secret: {} = <not found>", name);
        }
    }
    // Log startup message
    info!("=== Beyond The Horizon API Server Starting ===");

    // Get required secrets with proper error handling
    let session_secret = match secrets.get("SESSION_SECRET") {
        Some(secret) => secret,
        None => {
            // Log warning but use a default value instead of failing
            info!("SESSION_SECRET not found in secrets, using a default value");
            "default_session_secret_for_development_only".to_string()
        }
    };

    // Create a secret key for cookies
    let secret_key = Key::from(session_secret.as_bytes());

    // Get database URL
    let database_url = match secrets.get("DATABASE_URL") {
        Some(url) => url,
        None => {
            // Log warning but use a default value instead of failing
            error!("DATABASE_URL not found in secrets, using a default value");
            return Err(shuttle_runtime::Error::Custom(anyhow::anyhow!(
                "Database connection failed"
            )));
        }
    };

    // Connect to the database
    let pool = match PgPool::connect(&database_url).await {
        Ok(pool) => pool,
        Err(e) => {
            // This one should still fail as we can't proceed without a database
            error!("Failed to connect to Postgres: {}", e);
            return Err(shuttle_runtime::Error::Custom(anyhow::anyhow!(
                "Database connection failed"
            )));
        }
    };

    // Check database connection
    if handlers::db::check_db_connection(&pool).await {
        info!("Database connection established and verified");
    } else {
        info!("Database connection established but verification failed");
        // Continue execution instead of returning an error
    }

    info!("Starting BTH API Server with Shuttle...");

    // Create a configuration closure for Shuttle
    let config = move |cfg: &mut web::ServiceConfig| {
        // Configure CORS to be extremely permissive for testing
        let cors = Cors::default()
            .allowed_origin_fn(|_origin, _req_head| true)
            .allow_any_method()
            .allow_any_header()
            .expose_any_header()
            .supports_credentials()
            .max_age(3600);

        cfg.app_data(web::Data::new(pool.clone()));
        cfg.app_data(web::Data::new(session_secret.clone()));
        cfg.service(
            web::scope("")
                .wrap(Logger::new(
                    "%t [%s] \"%r\" %b %D ms \"%{Referer}i\" \"%{User-Agent}i\" %a",
                ))
                .wrap(RequestLogger)
                .wrap(cors)
                .wrap(IdentityMiddleware::default())
                .wrap(
                    SessionMiddleware::builder(CookieSessionStore::default(), secret_key.clone())
                        .cookie_secure(false) // Must be false for localhost testing
                        .cookie_http_only(true)
                        .cookie_same_site(SameSite::None) // Use None for cross-origin
                        .cookie_name("bth_session".to_string())
                        .cookie_path("/".to_string())
                        .build(),
                )
                .wrap(SessionRefreshMiddleware::new(30 * 60))
                .service(
                    web::scope("/api")
                        .service(web::scope("/public").configure(config_user_auth_routes))
                        .service(
                            web::scope("/protected")
                                .wrap(AuthMiddleware)
                                .configure(config_user_data_routes)
                                .configure(config_feed_routes)
                                .configure(config_message_routes)
                                .configure(config_matching_routes)
                                .configure(config_sponsor_routes)
                                .configure(config_support_group_routes)
                                .configure(config_meeting_routes)
                                .configure(config_group_chat_routes)
                                .configure(config_resource_routes)
                                .configure(config_report_routes)
                                .configure(init_ws_routes),
                        ),
                )
                .service(
                    web::scope("/api/admin")
                        .wrap(AuthMiddleware)
                        .configure(config_admin_routes),
                )
                .route(
                    "/",
                    web::get().to(|| async {
                        HttpResponse::Ok().body("Welcome to Beyond The Horizon API")
                    }),
                )
                .service(
                    web::resource("/api/test-cors").route(web::get().to(|| async {
                        info!("CORS test endpoint called");
                        HttpResponse::Ok().json(serde_json::json!({
                            "message": "CORS is working!",
                            "timestamp": chrono::Utc::now().to_rfc3339()
                        }))
                    })),
                )
                .route(
                    "/api/test-auth",
                    web::get().to(|| async {
                        HttpResponse::Ok()
                            .cookie(
                                Cookie::build("test_auth", "test_value")
                                    .path("/")
                                    .secure(true)
                                    .http_only(true)
                                    .same_site(SameSite::None)
                                    .finish(),
                            )
                            .json(serde_json::json!({
                                "message": "Test auth cookie set",
                                "timestamp": chrono::Utc::now().to_rfc3339()
                            }))
                    }),
                )
                .route(
                    "/api/test-cookie",
                    web::get().to(|| async {
                        HttpResponse::Ok()
                            .cookie(
                                Cookie::build("test_cookie", "test_value")
                                    .path("/")
                                    .http_only(false) // Make it visible to JavaScript
                                    .same_site(SameSite::Lax)
                                    .finish(),
                            )
                            .json(serde_json::json!({
                                "message": "Test cookie set",
                                "timestamp": chrono::Utc::now().to_rfc3339()
                            }))
                    }),
                ),
        );
    };

    // Return the configuration for Shuttle
    Ok(config.into())
}
