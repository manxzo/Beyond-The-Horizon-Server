use crate::handlers::auth::Claims;
use crate::handlers::ws;
use crate::models::all_models::{GroupChat, GroupChatMember, GroupChatMessage};
use actix_web::{HttpMessage, HttpRequest, HttpResponse, Responder, web};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

// -----------------------
// Request Structures
// -----------------------

//Send Group Chat Message Request
#[derive(Debug, Deserialize, Serialize)]
pub struct SendGroupChatMessageRequest {
    pub content: String,
}

//Add Group Chat Member Request
#[derive(Debug, Deserialize, Serialize)]
pub struct AddGroupChatMemberRequest {
    pub member_id: Uuid,
}

//Group Chat Invitation Request
#[derive(Debug, Deserialize, Serialize)]
pub struct GroupChatInvitationRequest {
    pub target_user_id: Uuid,
    pub message: String,
}

// Helper function: Check if a user is a member of a group chat.
async fn is_member(pool: &PgPool, group_chat_id: Uuid, user_id: Uuid) -> Result<bool, sqlx::Error> {
    let query = "SELECT COUNT(*) FROM group_chat_members WHERE group_chat_id = $1 AND user_id = $2";
    let count: i64 = sqlx::query_scalar(query)
        .bind(group_chat_id)
        .bind(user_id)
        .fetch_one(pool)
        .await?;
    Ok(count > 0)
}

// -----------------------
// Handler Implementations
// -----------------------

//Create Group Chat
//Create Group Chat Input: HttpRequest(JWT Token)
//Create Group Chat Output: GroupChat
pub async fn create_group_chat(pool: web::Data<PgPool>, req: HttpRequest) -> impl Responder {
    // Clone the Claims from the request for full ownership.
    let claims: Claims = match req.extensions().get::<Claims>() {
        Some(claims) => claims.clone(),
        None => return HttpResponse::Unauthorized().body("Authentication required"),
    };

    // Use the creator's user id for the new chat.
    let creator_id = claims.id;

    // Corrected SQL query to match the table structure
    let query = r#"
        INSERT INTO group_chats (creator_id, created_at, flagged)
        VALUES ($1, NOW(), false)
        RETURNING group_chat_id, creator_id, created_at
    "#;
    match sqlx::query_as::<_, GroupChat>(query)
        .bind(creator_id)
        .fetch_one(pool.get_ref())
        .await
    {
        Ok(chat) => HttpResponse::Ok().json(chat),
        Err(e) => {
            eprintln!("Error creating group chat: {:?}", e);
            HttpResponse::InternalServerError().body("Failed to create group chat")
        }
    }
}

//Get Group Chat Details
//Get Group Chat Details Input: HttpRequest(JWT Token), Path (/group-chats/{group_chat_id})
//Get Group Chat Details Output: ChatDetails
pub async fn get_group_chat_details(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<Uuid>, // group_chat_id
) -> impl Responder {
    let claims: Claims = match req.extensions().get::<Claims>() {
        Some(c) => c.clone(),
        None => return HttpResponse::Unauthorized().body("Authentication required"),
    };
    let group_chat_id = path.into_inner();
    let user_id = claims.id; // Using id directly as Uuid

    // Ensure the authenticated user is a member of this group chat.
    match is_member(pool.get_ref(), group_chat_id, user_id).await {
        Ok(false) => {
            return HttpResponse::Forbidden().body("You are not a member of this group chat");
        }
        Err(e) => {
            eprintln!("Error checking membership: {:?}", e);
            return HttpResponse::InternalServerError().body("Membership check failed");
        }
        _ => {}
    }

    // Retrieve the group chat record.
    let chat_query = r#"
        SELECT group_chat_id, creator_id, created_at
        FROM group_chats
        WHERE group_chat_id = $1
    "#;
    let chat = match sqlx::query_as::<_, GroupChat>(chat_query)
        .bind(group_chat_id)
        .fetch_one(pool.get_ref())
        .await
    {
        Ok(chat) => chat,
        Err(e) => {
            eprintln!("Error fetching group chat: {:?}", e);
            return HttpResponse::NotFound().body("Group chat not found");
        }
    };

    // Retrieve group chat members.
    let members_query = r#"
        SELECT group_chat_id, user_id
        FROM group_chat_members
        WHERE group_chat_id = $1
    "#;
    let members = match sqlx::query_as::<_, GroupChatMember>(members_query)
        .bind(group_chat_id)
        .fetch_all(pool.get_ref())
        .await
    {
        Ok(members) => members,
        Err(e) => {
            eprintln!("Error fetching group chat members: {:?}", e);
            Vec::new()
        }
    };

    // Retrieve group chat messages (ordered by timestamp).
    let messages_query = r#"
        SELECT group_chat_message_id, group_chat_id, sender_id, content, timestamp, deleted, edited
        FROM group_chat_messages
        WHERE group_chat_id = $1
        ORDER BY timestamp ASC
    "#;
    let messages = match sqlx::query_as::<_, GroupChatMessage>(messages_query)
        .bind(group_chat_id)
        .fetch_all(pool.get_ref())
        .await
    {
        Ok(messages) => messages,
        Err(e) => {
            eprintln!("Error fetching group chat messages: {:?}", e);
            Vec::new()
        }
    };

    //Chat Details
    #[derive(Debug, Serialize)]
    struct ChatDetails {
        chat: GroupChat,
        members: Vec<GroupChatMember>,
        messages: Vec<GroupChatMessage>,
    }

    let details = ChatDetails {
        chat,
        members,
        messages,
    };
    HttpResponse::Ok().json(details)
}

//List User Group Chats
//List User Group Chats Input: HttpRequest(JWT Token)
//List User Group Chats Output: Vec<GroupChat>
pub async fn list_user_group_chats(pool: web::Data<PgPool>, req: HttpRequest) -> impl Responder {
    let claims: Claims = match req.extensions().get::<Claims>() {
        Some(c) => c.clone(),
        None => return HttpResponse::Unauthorized().body("Authentication required"),
    };

    let user_id = claims.id;

    let query = r#"
        SELECT gc.group_chat_id, gc.creator_id, gc.created_at
        FROM group_chats gc
        JOIN group_chat_members gcm ON gc.group_chat_id = gcm.group_chat_id
        WHERE gcm.user_id = $1
        ORDER BY gc.created_at DESC
    "#;
    match sqlx::query_as::<_, GroupChat>(query)
        .bind(user_id)
        .fetch_all(pool.get_ref())
        .await
    {
        Ok(chats) => HttpResponse::Ok().json(chats),
        Err(e) => {
            eprintln!("Error listing user group chats: {:?}", e);
            HttpResponse::InternalServerError().body("Failed to list group chats")
        }
    }
}

//Send Group Chat Message
//Send Group Chat Message Input: HttpRequest(JWT Token), Path (/group-chats/{group_chat_id}/messages), SendGroupChatMessageRequest
//Send Group Chat Message Output: GroupChatMessage
pub async fn send_group_chat_message(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<Uuid>, // group_chat_id
    payload: web::Json<SendGroupChatMessageRequest>,
) -> impl Responder {
    let claims: Claims = match req.extensions().get::<Claims>() {
        Some(c) => c.clone(),
        None => return HttpResponse::Unauthorized().body("Authentication required"),
    };
    let sender_id = claims.id;
    let group_chat_id = path.into_inner();

    // Check if the sender is a member of the group chat.
    match is_member(pool.get_ref(), group_chat_id, sender_id).await {
        Ok(false) => {
            return HttpResponse::Forbidden().body("You are not a member of this group chat");
        }
        Err(e) => {
            eprintln!("Error checking membership: {:?}", e);
            return HttpResponse::InternalServerError().body("Membership check failed");
        }
        _ => {}
    }

    // Corrected SQL query to match the number of values with columns
    let query = r#"
        INSERT INTO group_chat_messages 
            (group_chat_id, sender_id, content, timestamp, deleted, edited)
        VALUES 
            ($1, $2, $3, NOW(), false, false)
        RETURNING group_chat_message_id, group_chat_id, sender_id, content, timestamp, deleted, edited
    "#;
    match sqlx::query_as::<_, GroupChatMessage>(query)
        .bind(group_chat_id)
        .bind(sender_id)
        .bind(&payload.content)
        .fetch_one(pool.get_ref())
        .await
    {
        Ok(message) => {
            // Send WebSocket notification to all members of the group chat
            let members_query = "SELECT user_id FROM group_chat_members WHERE group_chat_id = $1";
            if let Ok(members) = sqlx::query_scalar::<_, Uuid>(members_query)
                .bind(group_chat_id)
                .fetch_all(pool.get_ref())
                .await
            {
                let ws_payload = json!({
                    "type": "new_group_chat_message",
                    "message": message,
                });

                for member_id in members {
                    ws::send_to_user(&member_id, ws_payload.clone()).await;
                }
            }

            HttpResponse::Ok().json(message)
        }
        Err(e) => {
            eprintln!("Error sending group chat message: {:?}", e);
            HttpResponse::InternalServerError().body("Failed to send message")
        }
    }
}

//Edit Group Chat Message
//Edit Group Chat Message Input: HttpRequest(JWT Token), Path (/group-chats/{group_chat_id}/messages/{message_id}), SendGroupChatMessageRequest
//Edit Group Chat Message Output: GroupChatMessage
pub async fn edit_group_chat_message(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<(Uuid, Uuid)>, // (group_chat_id, message_id)
    payload: web::Json<SendGroupChatMessageRequest>,
) -> impl Responder {
    let claims: Claims = match req.extensions().get::<Claims>() {
        Some(c) => c.clone(),
        None => return HttpResponse::Unauthorized().body("Authentication required"),
    };
    let sender_id = claims.id;
    let (group_chat_id, message_id) = path.into_inner();
    let query = r#"
        UPDATE group_chat_messages 
        SET content = $1, edited = true
        WHERE group_chat_message_id = $2 AND sender_id = $3 AND group_chat_id = $4
        RETURNING group_chat_message_id, group_chat_id, sender_id, content, timestamp, deleted, edited
    "#;
    match sqlx::query_as::<_, GroupChatMessage>(query)
        .bind(&payload.content)
        .bind(message_id)
        .bind(sender_id)
        .bind(group_chat_id)
        .fetch_one(pool.get_ref())
        .await
    {
        Ok(message) => {
            // Send WebSocket notification to all members of the group chat
            let members_query = "SELECT user_id FROM group_chat_members WHERE group_chat_id = $1";
            if let Ok(members) = sqlx::query_scalar::<_, Uuid>(members_query)
                .bind(group_chat_id)
                .fetch_all(pool.get_ref())
                .await
            {
                let ws_payload = json!({
                    "type": "edited_group_chat_message",
                    "message": message,
                });

                for member_id in members {
                    ws::send_to_user(&member_id, ws_payload.clone()).await;
                }
            }

            HttpResponse::Ok().json(message)
        }
        Err(e) => {
            eprintln!("Error editing group chat message: {:?}", e);
            HttpResponse::InternalServerError().body("Failed to edit message")
        }
    }
}

//Delete Group Chat Message
//Delete Group Chat Message Input: HttpRequest(JWT Token), Path (/group-chats/{group_chat_id}/messages/{message_id})
//Delete Group Chat Message Output: Success message
pub async fn delete_group_chat_message(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<(Uuid, Uuid)>, // (group_chat_id, message_id)
) -> impl Responder {
    let claims: Claims = match req.extensions().get::<Claims>() {
        Some(c) => c.clone(),
        None => return HttpResponse::Unauthorized().body("Authentication required"),
    };
    let sender_id = claims.id;
    let (group_chat_id, message_id) = path.into_inner();
    let query = r#"
        UPDATE group_chat_messages 
        SET deleted = true
        WHERE group_chat_message_id = $1 AND sender_id = $2 AND group_chat_id = $3
        RETURNING group_chat_message_id, group_chat_id, sender_id, content, timestamp, deleted, edited
    "#;
    match sqlx::query_as::<_, GroupChatMessage>(query)
        .bind(message_id)
        .bind(sender_id)
        .bind(group_chat_id)
        .fetch_one(pool.get_ref())
        .await
    {
        Ok(message) => {
            // Send WebSocket notification to all members of the group chat
            let members_query = "SELECT user_id FROM group_chat_members WHERE group_chat_id = $1";
            if let Ok(members) = sqlx::query_scalar::<_, Uuid>(members_query)
                .bind(group_chat_id)
                .fetch_all(pool.get_ref())
                .await
            {
                let ws_payload = json!({
                    "type": "deleted_group_chat_message",
                    "message": message,
                });

                for member_id in members {
                    ws::send_to_user(&member_id, ws_payload.clone()).await;
                }
            }

            HttpResponse::Ok().json(message)
        }
        Err(e) => {
            eprintln!("Error deleting group chat message: {:?}", e);
            HttpResponse::InternalServerError().body("Failed to delete message")
        }
    }
}

//Add Group Chat Member
//Add Group Chat Member Input: HttpRequest(JWT Token), Path (/group-chats/{group_chat_id}/members), AddGroupChatMemberRequest
//Add Group Chat Member Output: GroupChatMember
pub async fn add_group_chat_member(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<Uuid>, // group_chat_id
    payload: web::Json<AddGroupChatMemberRequest>,
) -> impl Responder {
    let claims: Claims = match req.extensions().get::<Claims>() {
        Some(c) => c.clone(),
        None => return HttpResponse::Unauthorized().body("Authentication required"),
    };
    let group_chat_id = path.into_inner();
    let user_id = claims.id;

    // Check if the user is authorized to add members (e.g., is the creator)
    let auth_query = r#"
        SELECT creator_id FROM group_chats
        WHERE group_chat_id = $1
    "#;
    let creator_id: Option<Uuid> = match sqlx::query_scalar(auth_query)
        .bind(group_chat_id)
        .fetch_optional(pool.get_ref())
        .await
    {
        Ok(id) => id,
        Err(e) => {
            eprintln!("Error checking group chat creator: {:?}", e);
            return HttpResponse::InternalServerError().body("Failed to verify authorization");
        }
    };

    if creator_id.is_none() || creator_id.unwrap() != user_id {
        return HttpResponse::Forbidden()
            .body("Only the creator can add members to this group chat");
    }

    // Check if member already exists in the chat.
    let check_query = r#"
        SELECT COUNT(*) FROM group_chat_members
        WHERE group_chat_id = $1 AND user_id = $2
    "#;
    let count: i64 = sqlx::query_scalar(check_query)
        .bind(group_chat_id)
        .bind(payload.member_id)
        .fetch_one(pool.get_ref())
        .await
        .unwrap_or(0);
    if count > 0 {
        return HttpResponse::Conflict().body("Member already exists in group chat");
    }

    let insert_query = r#"
        INSERT INTO group_chat_members (group_chat_id, user_id)
        VALUES ($1, $2)
        RETURNING group_chat_id, user_id
    "#;
    match sqlx::query_as::<_, GroupChatMember>(insert_query)
        .bind(group_chat_id)
        .bind(payload.member_id)
        .fetch_one(pool.get_ref())
        .await
    {
        Ok(member) => {
            // Send WebSocket notification to all members of the group chat
            let members_query = "SELECT user_id FROM group_chat_members WHERE group_chat_id = $1";
            if let Ok(members) = sqlx::query_scalar::<_, Uuid>(members_query)
                .bind(group_chat_id)
                .fetch_all(pool.get_ref())
                .await
            {
                let ws_payload = json!({
                    "type": "member_added_to_group_chat",
                    "group_chat_id": group_chat_id,
                    "new_member_id": payload.member_id,
                });

                for member_id in members {
                    ws::send_to_user(&member_id, ws_payload.clone()).await;
                }
            }

            // Also notify the newly added member
            let ws_payload = json!({
                "type": "added_to_group_chat",
                "group_chat_id": group_chat_id,
            });
            ws::send_to_user(&payload.member_id, ws_payload).await;

            HttpResponse::Ok().json(member)
        }
        Err(e) => {
            eprintln!("Error adding member to group chat: {:?}", e);
            HttpResponse::InternalServerError().body("Failed to add member")
        }
    }
}

//Remove Group Chat Member
//Remove Group Chat Member Input: HttpRequest(JWT Token), Path (/group-chats/{group_chat_id}/members/{member_id})
//Remove Group Chat Member Output: Success message
pub async fn remove_group_chat_member(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<(Uuid, Uuid)>, // (group_chat_id, member_id)
) -> impl Responder {
    let claims: Claims = match req.extensions().get::<Claims>() {
        Some(c) => c.clone(),
        None => return HttpResponse::Unauthorized().body("Authentication required"),
    };
    let user_id = claims.id;
    let (group_chat_id, member_id) = path.into_inner();

    // Check if the user is authorized to remove members (is the creator or removing themselves)
    let auth_query = r#"
        SELECT creator_id FROM group_chats
        WHERE group_chat_id = $1
    "#;
    let creator_id: Option<Uuid> = match sqlx::query_scalar(auth_query)
        .bind(group_chat_id)
        .fetch_optional(pool.get_ref())
        .await
    {
        Ok(id) => id,
        Err(e) => {
            eprintln!("Error checking group chat creator: {:?}", e);
            return HttpResponse::InternalServerError().body("Failed to verify authorization");
        }
    };

    // Allow if user is removing themselves or is the creator
    if member_id != user_id && (creator_id.is_none() || creator_id.unwrap() != user_id) {
        return HttpResponse::Forbidden().body("You don't have permission to remove this member");
    }

    let query = r#"
        DELETE FROM group_chat_members
        WHERE group_chat_id = $1 AND user_id = $2
    "#;
    match sqlx::query(query)
        .bind(group_chat_id)
        .bind(member_id)
        .execute(pool.get_ref())
        .await
    {
        Ok(_) => {
            // Send WebSocket notification to all members of the group chat
            let members_query = "SELECT user_id FROM group_chat_members WHERE group_chat_id = $1";
            if let Ok(members) = sqlx::query_scalar::<_, Uuid>(members_query)
                .bind(group_chat_id)
                .fetch_all(pool.get_ref())
                .await
            {
                let ws_payload = json!({
                    "type": "member_removed_from_group_chat",
                    "group_chat_id": group_chat_id,
                    "removed_member_id": member_id,
                });

                for member_id in members {
                    ws::send_to_user(&member_id, ws_payload.clone()).await;
                }
            }

            // Also notify the removed member
            let ws_payload = json!({
                "type": "removed_from_group_chat",
                "group_chat_id": group_chat_id,
            });
            ws::send_to_user(&member_id, ws_payload).await;

            HttpResponse::Ok().body("Member removed from group chat")
        }
        Err(e) => {
            eprintln!("Error removing member from group chat: {:?}", e);
            HttpResponse::InternalServerError().body("Failed to remove member")
        }
    }
}

// -----------------------
// Route Configuration
// -----------------------

//Config Group Chat Routes
// POST /group-chats/new
// GET /group-chats/{group_chat_id}
// GET /group-chats
// POST /group-chats/{group_chat_id}/messages
// PATCH /group-chats/{group_chat_id}/messages/{message_id}
// DELETE /group-chats/{group_chat_id}/messages/{message_id}
// POST /group-chats/{group_chat_id}/members
// DELETE /group-chats/{group_chat_id}/members/{member_id}
pub fn config_group_chat_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/group-chats")
            .route("/create", web::post().to(create_group_chat))
            .route("/list", web::get().to(list_user_group_chats))
            .route("/{group_chat_id}", web::get().to(get_group_chat_details))
            .route(
                "/{group_chat_id}/messages",
                web::post().to(send_group_chat_message),
            )
            .route(
                "/{group_chat_id}/messages/{message_id}",
                web::patch().to(edit_group_chat_message),
            )
            .route(
                "/{group_chat_id}/messages/{message_id}",
                web::delete().to(delete_group_chat_message),
            )
            .route(
                "/{group_chat_id}/members",
                web::post().to(add_group_chat_member),
            )
            .route(
                "/{group_chat_id}/members/{member_id}",
                web::delete().to(remove_group_chat_member),
            ),
    );
}

