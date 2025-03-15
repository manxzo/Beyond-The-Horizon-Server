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
    session_secret: String,
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
                        match client_message.message_type.as_str() {
                            "authentication" => {
                                // Handle authentication message
                                if self.authenticated {
                                    info!("Already authenticated, ignoring authentication message");
                                    return;
                                }

                                match serde_json::from_value::<AuthMessage>(client_message.payload)
                                {
                                    Ok(auth_data) => {
                                        info!("Received authentication request");
                                        // Verify and decode the token
                                        match decode::<Claims>(
                                            &auth_data.token,
                                            &DecodingKey::from_secret(
                                                self.session_secret.as_bytes(),
                                            ),
                                            &Validation::default(),
                                        ) {
                                            Ok(token_data) => {
                                                let user_id = token_data.claims.id;
                                                let role = token_data.claims.role;

                                                info!(
                                                    "WebSocket authenticated: {} with role {:?}",
                                                    user_id, role
                                                );

                                                // Set up the session
                                                self.user_id = Some(user_id);
                                                self.role = Some(role);
                                                self.authenticated = true;

                                                // Set up the channel
                                                let (tx, rx): (
                                                    UnboundedSender<ws::Message>,
                                                    UnboundedReceiver<ws::Message>,
                                                ) = unbounded();
                                                self.tx = Some(tx.clone());

                                                // Register in the active connections
                                                {
                                                    let mut sockets = USER_SOCKETS.lock().unwrap();
                                                    sockets.insert(user_id, (role, tx));
                                                    info!(
                                                        "Active WebSocket connections: {}",
                                                        sockets.len()
                                                    );
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
                                            }
                                            Err(e) => {
                                                error!("WebSocket authentication failed: {}", e);
                                                let response = serde_json::json!({
                                                    "type": "authentication_error",
                                                    "payload": {
                                                        "message": "Invalid token"
                                                    }
                                                });
                                                ctx.text(serde_json::to_string(&response).unwrap());
                                                ctx.close(None);
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        error!("Failed to parse authentication payload: {}", e);
                                        let response = serde_json::json!({
                                            "type": "error",
                                            "payload": {
                                                "message": "Invalid authentication payload"
                                            }
                                        });
                                        ctx.text(serde_json::to_string(&response).unwrap());
                                    }
                                }
                            }
                            _ => {
                                // Handle other message types
                                if !self.authenticated {
                                    warn!(
                                        "Received message from unauthenticated client: {}",
                                        client_message.message_type
                                    );
                                    let response = serde_json::json!({
                                        "type": "error",
                                        "payload": {
                                            "message": "Not authenticated"
                                        }
                                    });
                                    ctx.text(serde_json::to_string(&response).unwrap());
                                    return;
                                }

                                // Echo the message for now
                                if let Some(user_id) = self.user_id {
                                    info!(
                                        "Received message from {}: {}",
                                        user_id, client_message.message_type
                                    );
                                    ctx.text(format!("Echo: {}", text));
                                }
                            }
                        }
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
            session_secret,
            authenticated: true,
        };

        // Start the WebSocket connection
        info!("Starting WebSocket connection for authenticated user");
        return ws::start(session, &req, stream);
    }

    // If we get here, the user is not authenticated
    // This should not happen with the auth middleware in place
    warn!("WebSocket connection attempt without authentication");

    // Create an unauthenticated session
    let session = WebSocketSession {
        user_id: None,
        role: None,
        tx: None,
        session_secret,
        authenticated: false,
    };

    // Start the WebSocket connection
    info!("Starting WebSocket connection for unauthenticated user");
    ws::start(session, &req, stream)
}

///  Send a payload to a single user
pub async fn send_to_user(user_id: &Uuid, payload: Value) {
    let msg_str = match serde_json::to_string(&payload) {
        Ok(s) => s,
        Err(_) => return,
    };
    let sockets = USER_SOCKETS.lock().unwrap();
    if let Some((_, tx)) = sockets.get(user_id) {
        let _ = tx.unbounded_send(ws::Message::Text(msg_str.clone().into()));
    }
}

///  Send a payload to all users with a specific role
pub async fn send_to_role(role: &UserRole, payload: Value) {
    let msg_str = match serde_json::to_string(&payload) {
        Ok(s) => s,
        Err(_) => return,
    };
    let sockets = USER_SOCKETS.lock().unwrap();
    for (_, (user_role, tx)) in sockets.iter() {
        if user_role == role {
            let _ = tx.unbounded_send(ws::Message::Text(msg_str.clone().into()));
        }
    }
}

///  Send a payload to multiple users
pub async fn send_to_users(user_ids: &[Uuid], payload: Value) {
    let msg_str = match serde_json::to_string(&payload) {
        Ok(s) => s,
        Err(_) => return,
    };
    let sockets = USER_SOCKETS.lock().unwrap();
    for user_id in user_ids {
        if let Some((_, tx)) = sockets.get(user_id) {
            let _ = tx.unbounded_send(ws::Message::Text(msg_str.clone().into()));
        }
    }
}

///  Send a payload to all users
pub async fn send_to_all(payload: Value) {
    let msg_str = match serde_json::to_string(&payload) {
        Ok(s) => s,
        Err(_) => return,
    };
    let sockets = USER_SOCKETS.lock().unwrap();
    for (_, (_, tx)) in sockets.iter() {
        let _ = tx.unbounded_send(ws::Message::Text(msg_str.clone().into()));
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
    send_to_user(&req.user_id, req.payload.clone()).await;
    HttpResponse::Ok().json("Message sent to specified user")
}

/// Handler to send a payload to all users with a specific role
pub async fn send_to_role_handler(req: web::Json<SendToRoleRequest>) -> impl Responder {
    send_to_role(&req.role, req.payload.clone()).await;
    HttpResponse::Ok().json("Message sent to users with specified role")
}

/// Handler to send a payload to multiple users
pub async fn send_to_users_handler(payload: web::Json<SendToUsersRequest>) -> impl Responder {
    send_to_users(&payload.user_ids, payload.payload.clone()).await;
    HttpResponse::Ok().json("Custom payload sent to specified users")
}

/// Handler to send a payload to all users
pub async fn send_to_all_handler(payload: web::Json<SendToAllRequest>) -> impl Responder {
    send_to_all(payload.payload.clone()).await;
    HttpResponse::Ok().json("Custom payload broadcasted to all users")
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
