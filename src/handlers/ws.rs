use crate::handlers::auth::Claims;
use crate::models::all_models::UserRole;
use actix::{Actor, AsyncContext, StreamHandler};
use actix_web::{Error, HttpMessage, HttpRequest, HttpResponse, Responder, web};
use actix_web_actors::ws;
use futures_channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use futures_util::StreamExt;
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

/// WebSocket session struct
struct WebSocketSession {
    user_id: Uuid,
    role: UserRole,
    tx: Option<UnboundedSender<ws::Message>>,
}

/// Shared map of active WebSocket connections.
type UserSocketMap = Arc<Mutex<HashMap<Uuid, (UserRole, UnboundedSender<ws::Message>)>>>;
lazy_static! {
    static ref USER_SOCKETS: UserSocketMap = Arc::new(Mutex::new(HashMap::new()));
}
impl Actor for WebSocketSession {
    type Context = ws::WebsocketContext<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        println!("WebSocket connected: {}", self.user_id);
        let (tx, rx): (UnboundedSender<ws::Message>, UnboundedReceiver<ws::Message>) = unbounded();
        self.tx = Some(tx.clone());
        {
            let mut sockets = USER_SOCKETS.lock().unwrap();
            sockets.insert(self.user_id, (self.role, tx));
        }
        ctx.add_stream(rx.map(|m| Ok(m)));
    }
    fn stopped(&mut self, _: &mut Self::Context) {
        println!("WebSocket disconnected: {}", self.user_id);
        USER_SOCKETS.lock().unwrap().remove(&self.user_id);
    }
}
impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for WebSocketSession {
    fn handle(&mut self, msg: Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
        if let Ok(ws::Message::Text(text)) = msg {
            println!("Received from {}: {}", self.user_id, text);
            ctx.text(format!("Echo: {}", text));
        }
    }
}

/// WebSocket connection handler
pub async fn ws_connect(req: HttpRequest, stream: web::Payload) -> Result<HttpResponse, Error> {
    if let Some(claims) = req.extensions().get::<Claims>() {
        let user_id = claims.id;
        let role = claims.role;
        let session = WebSocketSession {
            user_id,
            role,
            tx: None,
        };
        ws::start(session, &req, stream)
    } else {
        Ok(HttpResponse::Unauthorized().body("Authentication required"))
    }
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

#[derive(Deserialize,Serialize)]
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
