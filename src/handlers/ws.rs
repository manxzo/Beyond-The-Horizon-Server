use crate::handlers::auth::Claims;
use crate::models::all_models::UserRole;
use actix::{Actor, AsyncContext, StreamHandler};
use actix_web::{web, Error, HttpMessage, HttpRequest, HttpResponse, Responder};
use actix_web_actors::ws;
use futures_channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};
use futures_util::StreamExt;
use jsonwebtoken::{decode, DecodingKey, Validation};
use lazy_static::lazy_static;
use log::{debug, error, info, warn};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

/// WebSocket session struct
struct WebSocketSession {
    user_id: Option<Uuid>,
    role: Option<UserRole>,
    tx: Option<UnboundedSender<ws::Message>>,
    authenticated: bool,
}

/// Shared map of active WebSocket connections.
type UserSocketMap = Arc<Mutex<HashMap<Uuid, (UserRole, UnboundedSender<ws::Message>)>>>;
lazy_static! {
    static ref USER_SOCKETS: UserSocketMap = Arc::new(Mutex::new(HashMap::new()));
}

#[derive(Deserialize, Serialize)]
struct AuthMessage {
    token: String,
}

#[derive(Deserialize, Serialize)]
struct WebSocketClientMessage {
    #[serde(rename = "type")]
    message_type: String,
    #[serde(default)]
    payload: Value,
}

impl Actor for WebSocketSession {
    type Context = ws::WebsocketContext<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        if self.authenticated {
            if let (Some(user_id), Some(role)) = (self.user_id, self.role) {
                info!(
                    "WebSocket connection started for authenticated user: {}",
                    user_id
                );

                // Set up the channel
                let (tx, rx): (UnboundedSender<ws::Message>, UnboundedReceiver<ws::Message>) =
                    unbounded();
                self.tx = Some(tx.clone());

                // Register in the active connections
                {
                    let mut sockets = USER_SOCKETS.lock().unwrap();
                    sockets.insert(user_id, (role, tx));
                    info!("Active WebSocket connections: {}", sockets.len());
                }

                // Add the stream to the context
                ctx.add_stream(rx.map(|m| Ok(m)));

                // Send confirmation
                let response = serde_json::json!({
                    "type": "authentication_success",
                    "payload": {
                        "user_id": user_id.to_string(),
                        "role": role
                    }
                });
                info!("Sending authentication success response");
                ctx.text(serde_json::to_string(&response).unwrap());
            } else {
                error!("WebSocket session marked as authenticated but missing user_id or role");
                ctx.close(None);
            }
        } else {
            info!("WebSocket connection started, waiting for authentication");
        }
    }

    fn stopped(&mut self, _: &mut Self::Context) {
        if let Some(user_id) = self.user_id {
            info!("WebSocket disconnected: {}", user_id);
            USER_SOCKETS.lock().unwrap().remove(&user_id);
        } else {
            info!("Unauthenticated WebSocket disconnected");
        }
    }
}

impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for WebSocketSession {
    fn handle(&mut self, msg: Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
        match msg {
            Ok(ws::Message::Text(text)) => {
                debug!("Received text message: {}", text);
                // Try to parse the message
                match serde_json::from_str::<WebSocketClientMessage>(&text) {
                    Ok(client_message) => {
                        debug!("Parsed message type: {}", client_message.message_type);

                        // We no longer need to handle authentication messages since we authenticate via URL token
                        // Just handle regular messages
                        if !self.authenticated {
                            error!("Received message from unauthenticated client");
                            let response = serde_json::json!({
                                "type": "error",
                                "payload": {
                                    "message": "Not authenticated"
                                }
                            });
                            ctx.text(serde_json::to_string(&response).unwrap());
                            return;
                        }

                        // Handle other message types here
                        // ...
                    }
                    Err(e) => {
                        error!("Invalid message format: {}", e);
                        // Invalid message format
                        let response = serde_json::json!({
                            "type": "error",
                            "payload": {
                                "message": "Invalid message format"
                            }
                        });
                        ctx.text(serde_json::to_string(&response).unwrap());
                    }
                }
            }
            Ok(ws::Message::Ping(msg)) => {
                debug!("Ping received");
                ctx.pong(&msg);
            }
            Ok(ws::Message::Pong(_)) => {
                debug!("Pong received");
            }
            Ok(ws::Message::Binary(bin)) => {
                debug!("Binary message received, length: {}", bin.len());
            }
            Ok(ws::Message::Close(reason)) => {
                info!("Close message received: {:?}", reason);
                ctx.close(reason);
            }
            Ok(ws::Message::Continuation(_)) => {
                debug!("Continuation message received");
            }
            Ok(ws::Message::Nop) => {
                debug!("Nop message received");
            }
            Err(e) => {
                error!("Error in WebSocket message: {}", e);
            }
        }
    }
}

/// WebSocket connection handler
pub async fn ws_connect(req: HttpRequest, stream: web::Payload) -> Result<HttpResponse, Error> {
    info!("WebSocket connection request received");

    // Get the session secret
    let session_secret = req
        .app_data::<web::Data<String>>()
        .map(|data| data.get_ref().clone())
        .unwrap_or_else(|| {
            warn!("No session secret found, using default");
            "default_session_secret".to_string()
        });

    // Check if the user is already authenticated via the auth middleware
    if let Some(claims) = req.extensions().get::<Claims>() {
        info!(
            "User already authenticated via middleware: {}",
            claims.username
        );

        let user_id = claims.id;
        let role = claims.role;

        // Create an authenticated session
        let session = WebSocketSession {
            user_id: Some(user_id),
            role: Some(role),
            tx: None,
            authenticated: true,
        };

        // Start the WebSocket connection
        info!("Starting WebSocket connection for authenticated user");
        return ws::start(session, &req, stream);
    }

    // If we get here, the user is not authenticated via middleware
    // Try to authenticate using the WebSocket protocol
    if let Some(protocols) = req.headers().get("sec-websocket-protocol") {
        if let Ok(protocols_str) = protocols.to_str() {
            // Extract token from protocol
            for protocol in protocols_str.split(',').map(|s| s.trim()) {
                if protocol.starts_with("token-") {
                    let token = protocol.trim_start_matches("token-");
                    info!("Found token in WebSocket protocol");

                    // Verify the token
                    match decode::<Claims>(
                        token,
                        &DecodingKey::from_secret(session_secret.as_bytes()),
                        &Validation::default(),
                    ) {
                        Ok(token_data) => {
                            let user_id = token_data.claims.id;
                            let role = token_data.claims.role;

                            info!("WebSocket authenticated via protocol: {}", user_id);

                            // Create an authenticated session
                            let session = WebSocketSession {
                                user_id: Some(user_id),
                                role: Some(role),
                                tx: None,
                                authenticated: true,
                            };

                            // Start the WebSocket connection
                            info!("Starting WebSocket connection for authenticated user");
                            return ws::start(session, &req, stream);
                        }
                        Err(e) => {
                            error!("Invalid token in WebSocket protocol: {}", e);
                        }
                    }
                }
            }
        }
    }

    // If we get here, the user is not authenticated
    warn!("WebSocket connection attempt without valid authentication");

    // Create an unauthenticated session
    let session = WebSocketSession {
        user_id: None,
        role: None,
        tx: None,
        authenticated: false,
    };

    // Start the WebSocket connection
    info!("Starting WebSocket connection for unauthenticated user");
    ws::start(session, &req, stream)
}

///  Send a payload to a single user
pub async fn send_to_user(user_id: &Uuid, payload: Value) -> Result<(), String> {
    let msg_str = match serde_json::to_string(&payload) {
        Ok(s) => s,
        Err(e) => {
            error!("Failed to serialize payload for user {}: {}", user_id, e);
            return Err(format!("Serialization error: {}", e));
        }
    };

    let sockets = match USER_SOCKETS.lock() {
        Ok(guard) => guard,
        Err(e) => {
            error!("Failed to acquire lock on USER_SOCKETS: {}", e);
            return Err("Internal server error: Failed to acquire lock".to_string());
        }
    };

    if let Some((_, tx)) = sockets.get(user_id) {
        match tx.unbounded_send(ws::Message::Text(msg_str.into())) {
            Ok(_) => {
                debug!("Message sent successfully to user {}", user_id);
                Ok(())
            }
            Err(e) => {
                error!("Failed to send message to user {}: {}", user_id, e);
                Err(format!("Send error: {}", e))
            }
        }
    } else {
        warn!("User {} not connected", user_id);
        Err(format!("User {} not connected", user_id))
    }
}

///  Send a payload to all users with a specific role
pub async fn send_to_role(role: &UserRole, payload: Value) -> Result<usize, String> {
    let msg_str = match serde_json::to_string(&payload) {
        Ok(s) => s,
        Err(e) => {
            error!("Failed to serialize payload for role {:?}: {}", role, e);
            return Err(format!("Serialization error: {}", e));
        }
    };

    let sockets = match USER_SOCKETS.lock() {
        Ok(guard) => guard,
        Err(e) => {
            error!("Failed to acquire lock on USER_SOCKETS: {}", e);
            return Err("Internal server error: Failed to acquire lock".to_string());
        }
    };

    let mut success_count = 0;
    let mut errors = Vec::new();

    for (user_id, (user_role, tx)) in sockets.iter() {
        if user_role == role {
            match tx.unbounded_send(ws::Message::Text(msg_str.clone().into())) {
                Ok(_) => {
                    debug!(
                        "Message sent successfully to user {} with role {:?}",
                        user_id, role
                    );
                    success_count += 1;
                }
                Err(e) => {
                    let error_msg = format!(
                        "Failed to send message to user {} with role {:?}: {}",
                        user_id, role, e
                    );
                    error!("{}", error_msg);
                    errors.push(error_msg);
                }
            }
        }
    }

    if !errors.is_empty() && success_count == 0 {
        Err(format!(
            "Failed to send to any users with role {:?}: {}",
            role,
            errors.join(", ")
        ))
    } else {
        Ok(success_count)
    }
}

///  Send a payload to multiple users
pub async fn send_to_users(user_ids: &[Uuid], payload: Value) -> Result<usize, String> {
    let msg_str = match serde_json::to_string(&payload) {
        Ok(s) => s,
        Err(e) => {
            error!("Failed to serialize payload for multiple users: {}", e);
            return Err(format!("Serialization error: {}", e));
        }
    };

    let sockets = match USER_SOCKETS.lock() {
        Ok(guard) => guard,
        Err(e) => {
            error!("Failed to acquire lock on USER_SOCKETS: {}", e);
            return Err("Internal server error: Failed to acquire lock".to_string());
        }
    };

    let mut success_count = 0;
    let mut errors = Vec::new();

    for user_id in user_ids {
        if let Some((_, tx)) = sockets.get(user_id) {
            match tx.unbounded_send(ws::Message::Text(msg_str.clone().into())) {
                Ok(_) => {
                    debug!("Message sent successfully to user {}", user_id);
                    success_count += 1;
                }
                Err(e) => {
                    let error_msg = format!("Failed to send message to user {}: {}", user_id, e);
                    error!("{}", error_msg);
                    errors.push(error_msg);
                }
            }
        } else {
            let error_msg = format!("User {} not connected", user_id);
            warn!("{}", error_msg);
            errors.push(error_msg);
        }
    }

    if !errors.is_empty() && success_count == 0 {
        Err(format!(
            "Failed to send to any users: {}",
            errors.join(", ")
        ))
    } else {
        Ok(success_count)
    }
}

///  Send a payload to all users
pub async fn send_to_all(payload: Value) -> Result<usize, String> {
    let msg_str = match serde_json::to_string(&payload) {
        Ok(s) => s,
        Err(e) => {
            error!("Failed to serialize payload for broadcast: {}", e);
            return Err(format!("Serialization error: {}", e));
        }
    };

    let sockets = match USER_SOCKETS.lock() {
        Ok(guard) => guard,
        Err(e) => {
            error!("Failed to acquire lock on USER_SOCKETS: {}", e);
            return Err("Internal server error: Failed to acquire lock".to_string());
        }
    };

    let mut success_count = 0;
    let mut errors = Vec::new();

    for (user_id, (_, tx)) in sockets.iter() {
        match tx.unbounded_send(ws::Message::Text(msg_str.clone().into())) {
            Ok(_) => {
                debug!("Message sent successfully to user {}", user_id);
                success_count += 1;
            }
            Err(e) => {
                let error_msg = format!("Failed to send message to user {}: {}", user_id, e);
                error!("{}", error_msg);
                errors.push(error_msg);
            }
        }
    }

    if !errors.is_empty() && success_count == 0 {
        Err(format!(
            "Failed to broadcast to any users: {}",
            errors.join(", ")
        ))
    } else {
        Ok(success_count)
    }
}

// Request/Response structs for handlers
#[derive(Deserialize, Serialize)]
pub struct SendToUserRequest {
    pub user_id: Uuid,
    pub payload: Value,
}

#[derive(Deserialize, Serialize)]
pub struct SendToRoleRequest {
    pub role: UserRole,
    pub payload: Value,
}

#[derive(Deserialize, Serialize)]
pub struct SendToUsersRequest {
    pub user_ids: Vec<Uuid>,
    pub payload: Value,
}

#[derive(Deserialize, Serialize)]
pub struct SendToAllRequest {
    pub payload: Value,
}

// Handler functions for routes
/// Handler to send a payload to a single user
pub async fn send_to_user_handler(req: web::Json<SendToUserRequest>) -> impl Responder {
    match send_to_user(&req.user_id, req.payload.clone()).await {
        Ok(_) => HttpResponse::Ok().json("Message sent to specified user"),
        Err(e) => {
            error!("Failed to send message to user {}: {}", req.user_id, e);
            HttpResponse::InternalServerError().json(format!("Failed to send message: {}", e))
        }
    }
}

/// Handler to send a payload to all users with a specific role
pub async fn send_to_role_handler(req: web::Json<SendToRoleRequest>) -> impl Responder {
    match send_to_role(&req.role, req.payload.clone()).await {
        Ok(count) => HttpResponse::Ok().json(format!(
            "Message sent to {} users with specified role",
            count
        )),
        Err(e) => {
            error!("Failed to send message to role {:?}: {}", req.role, e);
            HttpResponse::InternalServerError().json(format!("Failed to send message: {}", e))
        }
    }
}

/// Handler to send a payload to multiple users
pub async fn send_to_users_handler(payload: web::Json<SendToUsersRequest>) -> impl Responder {
    match send_to_users(&payload.user_ids, payload.payload.clone()).await {
        Ok(count) => {
            HttpResponse::Ok().json(format!("Custom payload sent to {} specified users", count))
        }
        Err(e) => {
            error!("Failed to send message to multiple users: {}", e);
            HttpResponse::InternalServerError().json(format!("Failed to send message: {}", e))
        }
    }
}

/// Handler to send a payload to all users
pub async fn send_to_all_handler(payload: web::Json<SendToAllRequest>) -> impl Responder {
    match send_to_all(payload.payload.clone()).await {
        Ok(count) => {
            HttpResponse::Ok().json(format!("Custom payload broadcasted to {} users", count))
        }
        Err(e) => {
            error!("Failed to broadcast message: {}", e);
            HttpResponse::InternalServerError().json(format!("Failed to broadcast message: {}", e))
        }
    }
}

/// ws routes
pub fn init_ws_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/ws")
            .route("/connect", web::get().to(ws_connect))
            .route("/send-user", web::post().to(send_to_user_handler))
            .route("/send-users", web::post().to(send_to_users_handler))
            .route("/send-role", web::post().to(send_to_role_handler))
            .route("/send-all", web::post().to(send_to_all_handler)),
    );
}
