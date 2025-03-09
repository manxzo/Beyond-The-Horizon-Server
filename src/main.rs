mod handlers;
mod middleware;
mod models;
mod routes;

use actix_cors::Cors;
use actix_identity::IdentityMiddleware;
use actix_session::{storage::CookieSessionStore, SessionMiddleware};
use actix_web::{
    cookie::{Key, SameSite},
    http::header,
    middleware::Logger,
    web, HttpResponse,
};
use env_logger::Env;
use handlers::ws::init_ws_routes;
use log::info;
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
use shuttle_actix_web::ShuttleActixWeb;
use shuttle_runtime::SecretStore;
use sqlx::PgPool;
use std::env;

#[shuttle_runtime::main]
async fn main(
    #[shuttle_runtime::Secrets] secrets: SecretStore,
) -> ShuttleActixWeb<impl FnOnce(&mut web::ServiceConfig) + Send + Clone + 'static> {
    dotenvy::dotenv().ok();

    // Get session secret from secrets
    let session_secret = secrets.get("SESSION_SECRET").unwrap();
    println!("SESSION_SECRET: {}", session_secret);
    // Create a secret key for cookies from the session secret
    let secret_key = Key::from(session_secret.as_bytes());

    // Configure and initialize logger safely (won't panic if already initialized)
    let env = Env::default().filter_or("RUST_LOG", "info,actix_web=debug,serv=debug");
    env_logger::Builder::from_env(env)
        .format_timestamp_millis()
        .format_module_path(true)
        .try_init()
        .ok(); // Use try_init() and ignore errors if logger is already initialized

    // Log a startup message to verify logging is working
    info!("=== Beyond The Horizon API Server Starting ===");
    info!("Logging initialized at debug level for actix_web");

    // Log whether we're running in local or production mode
    if let Ok(port) = env::var("PORT") {
        info!("Starting BTH API Server with Shuttle on port {}...", port);
    } else {
        info!("Starting BTH API Server with Shuttle...");
    }

    // Get database URL from secrets
    let database_url = secrets
        .get("DATABASE_URL")
        .expect("DATABASE_URL not found in secrets");

    // Connect to the database using the connection string
    let pool = PgPool::connect(&database_url)
        .await
        .expect("Failed to connect to Postgres");

    // Check database connection
    if handlers::db::check_db_connection(&pool).await {
        info!("Database connection established and verified");
    } else {
        info!("Database connection established but verification failed");
    }

    // Get allowed origins from environment or use a default
    // In production, this should be your frontend domain
    let allowed_origins = env::var("ALLOWED_ORIGINS")
        .unwrap_or_else(|_| String::from("https://your-frontend-domain.com"));

    // Check if we're in development mode
    let is_development =
        env::var("ENVIRONMENT").unwrap_or_else(|_| "development".to_string()) == "development";

    // Convert to owned strings to avoid borrowing issues
    let origins: Vec<String> = allowed_origins
        .split(',')
        .map(|s| s.trim().to_owned())
        .collect();

    // Create a configuration closure for Shuttle
    let config = move |cfg: &mut web::ServiceConfig| {
        // Configure CORS
        let cors = Cors::default()
            .allowed_origin_fn(move |origin, _req_head| {
                // If no specific origins are defined, allow all
                if origins.is_empty() {
                    return true;
                }

                // Check if the origin is in our allowed list
                let origin_str = origin.to_str().unwrap_or("");
                origins.iter().any(|allowed| allowed == origin_str)
            })
            .allowed_methods(vec!["GET", "POST", "PUT", "DELETE", "PATCH"])
            .allowed_headers(vec![
                header::AUTHORIZATION,
                header::ACCEPT,
                header::CONTENT_TYPE,
                header::CONTENT_DISPOSITION, // Required for multipart form data
                header::CONTENT_LENGTH,      // Required for file uploads
                header::ORIGIN,              // Required for CORS
                header::ACCESS_CONTROL_REQUEST_METHOD,
                header::ACCESS_CONTROL_REQUEST_HEADERS,
            ])
            .expose_headers(vec![
                header::CONTENT_DISPOSITION,
                header::CONTENT_LENGTH,
                header::CONTENT_TYPE,
            ])
            .supports_credentials()
            .max_age(3600);

        cfg.app_data(web::Data::new(pool.clone()));
        cfg.service(
            web::scope("")
                .wrap(Logger::new(
                    "%t [%s] \"%r\" %b %D ms \"%{Referer}i\" \"%{User-Agent}i\" %a",
                ))
                .wrap(RequestLogger)
                // Add CORS middleware
                .wrap(cors)
                // Add Identity and Session middleware
                .wrap(IdentityMiddleware::default())
                .wrap(
                    SessionMiddleware::builder(CookieSessionStore::default(), secret_key.clone())
                        .cookie_secure(!is_development) // True in production, false in development
                        .cookie_http_only(true)
                        .cookie_same_site(SameSite::None) // None for cross-domain, Lax for same domain
                        .cookie_name("bth_session".to_string()) // Custom cookie name
                        .cookie_domain(if is_development {
                            None
                        } else {
                            Some("your-api-domain.com".to_string())
                        }) // Set to your API domain in production
                        .build(),
                )
                // Add session refresh middleware (refresh if less than 30 minutes left)
                .wrap(SessionRefreshMiddleware::new(30 * 60))
                // API routes
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
                // Admin routes
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
                ),
        );
    };

    // Return the configuration for Shuttle
    Ok(config.into())
}
