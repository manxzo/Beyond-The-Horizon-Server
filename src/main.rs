mod handlers;
mod middleware;
mod models;
mod routes;

use actix_identity::IdentityMiddleware;
use actix_session::{storage::CookieSessionStore, SessionMiddleware};
use actix_web::{
    cookie::{Key, SameSite},
    middleware::Logger,
    web, HttpResponse,
};
use env_logger::Env;
use handlers::ws::init_ws_routes;
use log::info;
use middleware::{auth_middleware::AuthMiddleware, request_logger::RequestLogger};
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

    // Get JWT secret from secrets
    let jwt_secret = secrets.get("JWT_SECRET").unwrap_or_else(|| {
        info!("JWT_SECRET not found in secrets, using default");
        "default_jwt_secret".to_string()
    });

    // Set JWT_SECRET environment variable for use in the application
    env::set_var("JWT_SECRET", jwt_secret.clone());

    // Get database URL from secrets or environment
    let database_url = secrets.get("DATABASE_URL").unwrap_or_else(|| {
        info!("DATABASE_URL not found in secrets, using environment variable");
        env::var("DATABASE_URL").expect("DATABASE_URL must be set in environment")
    });

    // Configure and initialize logger
    let env = Env::default().filter_or("RUST_LOG", "info,actix_web=info,serv=debug");
    env_logger::Builder::from_env(env)
        .format_timestamp_millis()
        .format_module_path(true)
        .init();

    // Log whether we're running in local or production mode
    if let Ok(port) = env::var("PORT") {
        info!("Starting BTH API Server with Shuttle on port {}...", port);
    } else {
        info!("Starting BTH API Server with Shuttle...");
    }

    // Connect to the database using the connection string
    let db_pool = PgPool::connect(&database_url)
        .await
        .expect("Failed to connect to Postgres");

    // Check database connection
    if handlers::db::check_db_connection(&db_pool).await {
        info!("Database connection established and verified");
    } else {
        info!("Database connection established but verification failed");
    }

    // Create a secret key for cookies from JWT secret
    let secret_key = Key::from(jwt_secret.as_bytes());

    // Create a configuration closure for Shuttle
    let config = move |cfg: &mut web::ServiceConfig| {
        cfg.app_data(web::Data::new(db_pool.clone()));
        cfg.service(
            web::scope("")
                .wrap(Logger::new(
                    "%a \"%r\" %s %b \"%{Referer}i\" \"%{User-Agent}i\" %T",
                ))
                .wrap(RequestLogger)
                // Add Identity and Session middleware
                .wrap(IdentityMiddleware::default())
                .wrap(
                    SessionMiddleware::builder(CookieSessionStore::default(), secret_key.clone())
                        .cookie_secure(true)
                        .cookie_http_only(true)
                        .cookie_same_site(SameSite::Lax)
                        .build(),
                )
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
                // Health check endpoint
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
