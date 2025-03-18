use crate::handlers::auth::Claims;
use crate::models::all_models::{Message, Report, ReportStatus, ReportedType};
use actix_web::{web, HttpMessage, HttpRequest, HttpResponse, Responder};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

//Send Message Request
#[derive(Debug, Deserialize, Serialize)]
pub struct SendMessageRequest {
    pub receiver_username: String,
    pub content: String,
}

//Send Message
//Send Message Input: HttpRequest(JWT Token), SendMessageRequest
//Send Message Output: Message
pub async fn send_message(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    payload: web::Json<SendMessageRequest>,
) -> impl Responder {
    if let Some(claims) = req.extensions().get::<Claims>() {
        let sender_id = claims.id;

        let receiver_result =
            sqlx::query_scalar::<_, Uuid>("SELECT user_id FROM users WHERE username = $1")
                .bind(&payload.receiver_username)
                .fetch_optional(pool.get_ref())
                .await;
        let receiver_id = match receiver_result {
            Ok(Some(id)) => id,
            Ok(None) => return HttpResponse::NotFound().body("Receiver not found"),
            Err(e) => {
                eprintln!("DB error: {:?}", e);
                return HttpResponse::InternalServerError().body("Database error");
            }
        };

        let insert_query = "
            INSERT INTO messages (sender_id, receiver_id, content, timestamp, deleted, edited)
            VALUES ($1, $2, $3, NOW(), false, false)
            RETURNING message_id, sender_id, receiver_id, content, timestamp, deleted, edited, seen_at
        ";
        let message_result = sqlx::query_as::<_, Message>(insert_query)
            .bind(sender_id)
            .bind(receiver_id)
            .bind(&payload.content)
            .fetch_one(pool.get_ref())
            .await;

        match message_result {
            Ok(message) => HttpResponse::Ok().json(message),
            Err(e) => {
                eprintln!("Error inserting message: {:?}", e);
                HttpResponse::InternalServerError().body("Failed to send message")
            }
        }
    } else {
        HttpResponse::Unauthorized().body("Authentication required")
    }
}

//Conversation List
#[derive(Debug, Serialize)]
pub struct ConversationList {
    pub usernames: Vec<String>,
}

//Get Conversation List
//Get Conversation List Input: HttpRequest(JWT Token)
//Get Conversation List Output: ConversationList
pub async fn get_conversation_list(pool: web::Data<PgPool>, req: HttpRequest) -> impl Responder {
    if let Some(claims) = req.extensions().get::<Claims>() {
        let user_id = claims.id;

        let query = r#"
            SELECT username FROM (
                SELECT receiver_id as other_id FROM messages WHERE sender_id = $1
                UNION
                SELECT sender_id as other_id FROM messages WHERE receiver_id = $1
            ) interactions
            JOIN users u ON interactions.other_id = u.user_id
        "#;

        match sqlx::query_scalar::<_, String>(query)
            .bind(user_id)
            .fetch_all(pool.get_ref())
            .await
        {
            Ok(usernames) => {
                let response = ConversationList { usernames };
                HttpResponse::Ok().json(response)
            }
            Err(e) => {
                eprintln!("Error fetching interaction usernames: {:?}", e);
                HttpResponse::InternalServerError().body("Failed to fetch interaction usernames")
            }
        }
    } else {
        HttpResponse::Unauthorized().body("Authentication required")
    }
}

//Get Conversation
//Get Conversation Input: HttpRequest(JWT Token), Path (/messages/{username})
//Get Conversation Output: Vec<Message>
pub async fn get_conversation(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<String>, // the partner's username
) -> impl Responder {
    if let Some(claims) = req.extensions().get::<Claims>() {
        let user_id = claims.id;

        let partner_username = path.into_inner();

        let partner_result =
            sqlx::query_scalar::<_, Uuid>("SELECT user_id FROM users WHERE username = $1")
                .bind(&partner_username)
                .fetch_optional(pool.get_ref())
                .await;
        let partner_id = match partner_result {
            Ok(Some(id)) => id,
            Ok(None) => return HttpResponse::NotFound().body("Partner not found"),
            Err(e) => {
                eprintln!("DB error: {:?}", e);
                return HttpResponse::InternalServerError().body("Database error");
            }
        };

        let query = "
            SELECT * FROM messages 
            WHERE (sender_id = $1 AND receiver_id = $2) OR (sender_id = $2 AND receiver_id = $1)
            AND deleted = false
            ORDER BY timestamp ASC
        ";
        let messages = sqlx::query_as::<_, Message>(query)
            .bind(user_id)
            .bind(partner_id)
            .fetch_all(pool.get_ref())
            .await;
        match messages {
            Ok(msgs) => HttpResponse::Ok().json(msgs),
            Err(e) => {
                eprintln!("Error fetching conversation: {:?}", e);
                HttpResponse::InternalServerError().body("Failed to fetch conversation")
            }
        }
    } else {
        HttpResponse::Unauthorized().body("Authentication required")
    }
}

//Mark Message Seen
//Mark Message Seen Input: HttpRequest(JWT Token), Path (/messages/seen/{message_id})
//Mark Message Seen Output: Message
pub async fn mark_message_seen(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<Uuid>, // message_id
) -> impl Responder {
    if let Some(claims) = req.extensions().get::<Claims>() {
        let user_id = claims.id;
        let message_id = path.into_inner();
        let query = "
            UPDATE messages 
            SET seen_at = NOW() 
            WHERE message_id = $1 AND receiver_id = $2
            RETURNING message_id, sender_id, receiver_id, content, timestamp, deleted, edited, seen_at
        ";
        let result = sqlx::query_as::<_, Message>(query)
            .bind(message_id)
            .bind(user_id)
            .fetch_one(pool.get_ref())
            .await;
        match result {
            Ok(message) => HttpResponse::Ok().json(message),
            Err(e) => {
                eprintln!("Error marking message as seen: {:?}", e);
                HttpResponse::InternalServerError().body("Failed to mark as seen")
            }
        }
    } else {
        HttpResponse::Unauthorized().body("Authentication required")
    }
}

//Edit Message Request
#[derive(Debug, Deserialize, Serialize)]
pub struct EditMessageRequest {
    pub content: String,
}

//Edit Message
//Edit Message Input: HttpRequest(JWT Token), Path (/messages/{message_id}), EditMessageRequest
//Edit Message Output: Message
pub async fn edit_message(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<Uuid>, // message_id
    payload: web::Json<EditMessageRequest>,
) -> impl Responder {
    if let Some(claims) = req.extensions().get::<Claims>() {
        let user_id = claims.id;
        let message_id = path.into_inner();
        let query = "
            UPDATE messages 
            SET content = $1, edited = true
            WHERE message_id = $2 AND sender_id = $3
            RETURNING message_id, sender_id, receiver_id, content, timestamp, deleted, edited, seen_at
        ";
        let result = sqlx::query_as::<_, Message>(query)
            .bind(&payload.content)
            .bind(message_id)
            .bind(user_id)
            .fetch_one(pool.get_ref())
            .await;
        match result {
            Ok(message) => HttpResponse::Ok().json(message),
            Err(e) => {
                eprintln!("Error editing message: {:?}", e);
                HttpResponse::InternalServerError().body("Failed to edit message")
            }
        }
    } else {
        HttpResponse::Unauthorized().body("Authentication required")
    }
}

//Delete Message
//Delete Message Input: HttpRequest(JWT Token), Path (/messages/{message_id})
//Delete Message Output: Success message
pub async fn delete_message(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<Uuid>, // message_id
) -> impl Responder {
    if let Some(claims) = req.extensions().get::<Claims>() {
        let user_id = claims.id;
        let message_id = path.into_inner();
        let query = "
            UPDATE messages 
            SET deleted = true
            WHERE message_id = $1 AND sender_id = $2
            RETURNING message_id, sender_id, receiver_id, content, timestamp, deleted, edited, seen_at
        ";
        let result = sqlx::query_as::<_, Message>(query)
            .bind(message_id)
            .bind(user_id)
            .fetch_one(pool.get_ref())
            .await;
        match result {
            Ok(message) => HttpResponse::Ok().json(message),
            Err(e) => {
                eprintln!("Error deleting message: {:?}", e);
                HttpResponse::InternalServerError().body("Failed to delete message")
            }
        }
    } else {
        HttpResponse::Unauthorized().body("Authentication required")
    }
}

//Report Message Request
#[derive(Debug, Deserialize, Serialize)]
pub struct ReportMessageRequest {
    pub reason: String,
}

//Report Message
//Report Message Input: HttpRequest(JWT Token), Path (/messages/report/{message_id}), ReportMessageRequest
//Report Message Output: Report
pub async fn report_message(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<Uuid>, // message_id
    payload: web::Json<ReportMessageRequest>,
) -> impl Responder {
    if let Some(claims) = req.extensions().get::<Claims>() {
        let reporter_id = claims.id;
        let message_id = path.into_inner();

        // First check if the message exists and get the sender_id
        let message_query = "SELECT sender_id FROM messages WHERE message_id = $1";
        let sender_id = match sqlx::query_scalar::<_, Uuid>(message_query)
            .bind(message_id)
            .fetch_optional(pool.get_ref())
            .await
        {
            Ok(Some(id)) => id,
            Ok(None) => return HttpResponse::NotFound().body("Message not found"),
            Err(e) => {
                eprintln!("DB error: {:?}", e);
                return HttpResponse::InternalServerError().body("Database error");
            }
        };

        // Create the report
        let insert_query = "
            INSERT INTO reports (reporter_id, reported_user_id, reported_item_id, reported_type, reason, status, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, NOW())
            RETURNING report_id, reporter_id, reported_user_id, reason, reported_type, reported_item_id, status, reviewed_by, resolved_at, created_at
        ";

        let report_result = sqlx::query_as::<_, Report>(insert_query)
            .bind(reporter_id)
            .bind(sender_id)
            .bind(message_id)
            .bind(ReportedType::Message)
            .bind(&payload.reason)
            .bind(ReportStatus::Pending)
            .fetch_one(pool.get_ref())
            .await;

        match report_result {
            Ok(report) => {
                // Convert the enum to string before serializing to JSON
                let response = json!({
                    "report_id": report.report_id,
                    "reporter_id": report.reporter_id,
                    "reported_user_id": report.reported_user_id,
                    "reason": report.reason,
                    "reported_type": format!("{:?}", report.reported_type),
                    "reported_item_id": report.reported_item_id,
                    "status": format!("{:?}", report.status),
                    "reviewed_by": report.reviewed_by,
                    "resolved_at": report.resolved_at,
                    "created_at": report.created_at
                });
                HttpResponse::Ok().json(response)
            }
            Err(e) => {
                eprintln!("Error creating report: {:?}", e);
                HttpResponse::InternalServerError().body("Failed to create report")
            }
        }
    } else {
        HttpResponse::Unauthorized().body("Authentication required")
    }
}

//Config Message Routes
// POST /messages/send
// GET /messages/conversations
// GET /messages/{username}
// PATCH /messages/seen/{message_id}
// PATCH /messages/{message_id}
// DELETE /messages/{message_id}
// POST /messages/report/{message_id}
pub fn config_message_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/messages")
            .route("/send", web::post().to(send_message))
            .route("/conversations", web::get().to(get_conversation_list))
            .route("/conversation/{username}", web::get().to(get_conversation))
            .route("/{message_id}/seen", web::put().to(mark_message_seen))
            .route("/{message_id}/edit", web::put().to(edit_message))
            .route("/{message_id}/report", web::post().to(report_message))
            .route("/{message_id}", web::delete().to(delete_message)),
    );
}
