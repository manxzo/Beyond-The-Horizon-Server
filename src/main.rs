mod handlers;
mod middleware;
mod models;
mod routes;

use actix_web::{App, HttpServer, middleware::Logger, web};
use env_logger::Env;
use handlers::{db::connect_db, ws::init_ws_routes};
use log::{debug, info};
use middleware::{auth_middleware::AuthMiddleware, request_logger::RequestLogger};
use routes::{
    admin::config_admin_routes, group_chats::config_group_chat_routes, posts::config_feed_routes,
    private_messaging::config_message_routes, report::config_report_routes,
    resources::config_resource_routes, sponsor_matching::config_matching_routes,
    sponsor_role::config_sponsor_routes, support_group_meetings::config_meeting_routes,
    support_groups::config_support_group_routes, user_auth::config_user_auth_routes,
    user_data::config_user_data_routes,
};
use std::env;
use std::io::Result as IoResult;
#[actix_web::main]
async fn main() -> IoResult<()> {
    // Initialize environment variables
    dotenvy::dotenv().ok();
    // Configure and initialize logger
    let env = Env::default().filter_or("RUST_LOG", "info,actix_web=info,serv=debug");
    env_logger::Builder::from_env(env)
        .format_timestamp_millis()
        .format_module_path(true)
        .init();
    info!("Starting BTH API Server...");
    // Log environment information
    let host = env::var("HOST").unwrap_or_else(|_| "localhost".to_string());
    let port = env::var("PORT").unwrap_or_else(|_| "3000".to_string());
    let bind_address = format!("{}:{}", host, port);
    info!("Environment Configuration:");
    info!("   - Binding to: {}", bind_address);
    debug!(
        "   - Log level: {}",
        env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string())
    );

    // Connect to database
    let pool = connect_db().await;
    info!("Database connection established");

    // Start HTTP server
    info!("Starting HTTP server at http://{}", bind_address);
    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(pool.clone()))
            // Add logger middleware
            .wrap(Logger::new(
                "%a \"%r\" %s %b \"%{Referer}i\" \"%{User-Agent}i\" %T",
            ))
            .wrap(RequestLogger)
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
            // Add admin routes with admin middleware
            .service(
                web::scope("/api/admin")
                    .wrap(AuthMiddleware)
                    .configure(config_admin_routes),
            )
    })
    .bind(&bind_address)?
    .run()
    .await
}

