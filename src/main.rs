mod handlers;
mod middleware;
mod models;
mod routes;

use actix_web::{App, HttpServer, middleware::Logger, web};
use env_logger::Env;
use handlers::db::connect_db;
use log::{debug, info};
use middleware::{auth_middleware::AuthMiddleware, request_logger::RequestLogger};
use routes::{
    posts::config_feed_routes, private_messaging::config_message_routes,
    sponsor_matching::config_matching_routes, sponsor_role::config_sponsor_routes,
    support_group_meetings::config_meeting_routes, support_groups::config_support_group_routes,
    user_auth::config_user_auth_routes, user_data::config_user_data_routes,
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
                    .service(web::scope("/public")
                        .configure(config_user_auth_routes))
                    .service(
                        web::scope("/protected")
                            .wrap(AuthMiddleware)
                            .configure(config_user_data_routes)
                            .configure(config_feed_routes)
                            .configure(config_message_routes)
                            .configure(config_matching_routes)
                            .configure(config_sponsor_routes)
                            .configure(config_support_group_routes)
                            .configure(config_meeting_routes),
                    ),
            )
    })
    .bind(&bind_address)?
    .run()
    .await
}

/*public routes*/
/*user auth routes*/
// POST /users/create
// POST /users/login

/* protected routes */
/* user info routes */
// GET /users/info
// GET /users/{username}
// PATCH /users/update-info
// DELETE /users/delete-user

/* sponsor routes */
// POST /sponsor/apply
// GET /sponsor/check
// PATCH /sponsor/update
// DELETE /sponsor/delete

/* support group routes */
// POST /support-groups/suggest
// GET /support-groups/list
// GET /support-groups/{group_id}
// POST /support-groups/join
// DELETE /support-groups/leave
// GET /support-groups/my

/* meeting routes */
// POST /meetings/{meeting_id}/join
// DELETE /meetings/{meeting_id}/leave
// GET /meetings/{meeting_id}/participants
// POST /meetings/{meeting_id}/start
// POST /meetings/{meeting_id}/end

/*resource routes*/
// GET /resources/list
// POST /resources/create
// GET /resources/{id}
// PATCH /resources/{id}
// DELETE /resources/{id}

/*report routes*/
// POST /reports/new

/* private messaging routes */
// POST /messages/send
// GET /messages/conversations
// GET /messages/conversation/{username}
// PUT /messages/{message_id}/seen
// PUT /messages/{message_id}/edit
// DELETE /messages/{message_id}

/* feed routes */
// GET /feed/posts/list
// POST /feed/posts/new
// GET /feed/posts/{id}
// PATCH /feed/posts/{id}
// DELETE /feed/posts/{id}
// GET /feed/posts/recent
// GET /feed/posts/search
// POST /feed/posts/like
// POST /feed/posts/unlike
// POST /feed/comments
// GET /feed/posts/{post_id}/comments
// PATCH /feed/comments/{id}
// DELETE /feed/comments/{id}

/* matching routes */
// GET /matching/recommend
// POST /matching/request
// GET /matching/status
// PATCH /matching/respond

/* group chat routes */
// POST /group-chats/create
// GET /group-chats/list
// GET /group-chats/{group_chat_id}
// POST /group-chats/{group_chat_id}/messages
// PATCH /group-chats/{group_chat_id}/messages/{message_id}
// DELETE /group-chats/{group_chat_id}/messages/{message_id}
// POST /group-chats/{group_chat_id}/members
// DELETE /group-chats/{group_chat_id}/members/{member_id}
