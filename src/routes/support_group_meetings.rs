use crate::handlers::auth::Claims;
use crate::handlers::ws;
use crate::models::all_models::{GroupMeeting,MeetingParticipant,GroupChat};
use actix_web::{HttpMessage, HttpRequest, HttpResponse, Responder, web};
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

// Create Support Group Meeting Request
#[derive(Debug, Deserialize, Serialize)]
pub struct CreateSupportGroupMeetingRequest {
    pub support_group_id: Uuid,
    pub title: String,
    pub description: Option<String>,
    pub scheduled_time: NaiveDateTime,
}

// Create Support Group Meeting Handler
// Create Support Group Meeting Input: CreateSupportGroupMeetingRequest
// Create Support Group Meeting Output: GroupMeeting
pub async fn create_support_group_meeting(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    payload: web::Json<CreateSupportGroupMeetingRequest>,
) -> impl Responder {
    if let Some(claims) = req.extensions().get::<Claims>() {
        // Ensure the support group exists and is approved, and get its group_chat_id.
        let sg_query = "
            SELECT group_chat_id FROM support_groups 
            WHERE support_group_id = $1 AND status = 'approved'
        ";
        let group_chat_id: Option<Uuid> = sqlx::query_scalar(sg_query)
            .bind(payload.support_group_id)
            .fetch_optional(pool.get_ref())
            .await
            .unwrap_or(None);
        if group_chat_id.is_none() {
            return HttpResponse::BadRequest().body("Support group not found or not approved");
        }
        let group_chat_id = group_chat_id.unwrap();

        let query = "
            INSERT INTO group_meetings (group_chat_id, host_id, title, description, scheduled_time)
            VALUES ($1, $2, $3, $4, $5)
            RETURNING meeting_id, group_chat_id, host_id, title, description, scheduled_time
        ";
        let meeting = sqlx::query_as::<_, GroupMeeting>(query)
            .bind(group_chat_id)
            .bind(claims.id) 
            .bind(&payload.title)
            .bind(&payload.description)
            .bind(&payload.scheduled_time)
            .fetch_one(pool.get_ref())
            .await;
        match meeting {
            Ok(m) => {
                
                let member_query =
                    "SELECT user_id FROM support_group_members WHERE support_group_id = $1";
                let member_ids: Vec<Uuid> = sqlx::query_scalar(member_query)
                    .bind(payload.support_group_id)
                    .fetch_all(pool.get_ref())
                    .await
                    .unwrap_or_default();
                let ws_payload = json!({
                    "type": "new_meeting",
                    "meeting": m,
                });
                for member_id in member_ids {
                    ws::send_to_user(&member_id, ws_payload.clone()).await;
                }
                HttpResponse::Ok().json(m)
            }
            Err(e) => {
                eprintln!("Error creating meeting: {:?}", e);
                HttpResponse::InternalServerError().body("Failed to create meeting")
            }
        }
    } else {
        HttpResponse::Unauthorized().body("Authentication required")
    }
}

// Join Meeting Request
#[derive(Debug, Deserialize, Serialize)]
pub struct JoinMeetingRequest {
    pub meeting_id: Uuid,
}

// Join Meeting Handler
// Join Meeting Input: JoinMeetingRequest
// Join Meeting Output: MeetingParticipant
pub async fn join_meeting(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    payload: web::Json<JoinMeetingRequest>,
) -> impl Responder {
    if let Some(claims) = req.extensions().get::<Claims>() {
        let user_id = claims.id; // Claims.id is already a Uuid
        let query = "
            INSERT INTO meeting_participants (meeting_id, user_id)
            VALUES ($1, $2)
            RETURNING meeting_id, user_id
        ";
        let participant = sqlx::query(query)
            .bind(payload.meeting_id)
            .bind(user_id)
            .fetch_one(pool.get_ref())
            .await;
        match participant {
            Ok(_row) => {
                // Optionally notify the host that a new participant has joined.
                let host_query = "SELECT host_id FROM group_meetings WHERE meeting_id = $1";
                if let Ok(host_id) = sqlx::query_scalar::<_, Uuid>(host_query)
                    .bind(payload.meeting_id)
                    .fetch_one(pool.get_ref())
                    .await
                {
                    let ws_payload = json!({
                        "type": "meeting_joined",
                        "meeting_id": payload.meeting_id,
                        "user_id": user_id,
                    });
                    ws::send_to_user(&host_id, ws_payload).await;
                }
                HttpResponse::Ok()
                    .json(json!({"meeting_id": payload.meeting_id, "user_id": user_id}))
            }
            Err(e) => {
                eprintln!("Error joining meeting: {:?}", e);
                HttpResponse::InternalServerError().body("Failed to join meeting")
            }
        }
    } else {
        HttpResponse::Unauthorized().body("Authentication required")
    }
}

// Leave Meeting Handler
// Leave Meeting Input: LeaveMeetingRequest
// Leave Meeting Output: String
pub async fn leave_meeting(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<Uuid>, // meeting_id provided in the URL
) -> impl Responder {
    // Check for authentication.
    if let Some(claims) = req.extensions().get::<Claims>() {
        // Get the user's UUID from the JWT claims.
        let user_id = claims.id; // Claims.id is already a Uuid
        let meeting_id = path.into_inner();

        // Delete the participant record from the meeting_participants table.
        let delete_query = "DELETE FROM meeting_participants WHERE meeting_id = $1 AND user_id = $2";
        let result = sqlx::query(delete_query)
            .bind(meeting_id)
            .bind(user_id)
            .execute(pool.get_ref())
            .await;

        match result {
            Ok(_) => {
                // Optionally, fetch the meeting host and notify them that the user left.
                let host_query = "SELECT host_id FROM group_meetings WHERE meeting_id = $1";
                if let Ok(host_id) = sqlx::query_scalar::<_, Uuid>(host_query)
                    .bind(meeting_id)
                    .fetch_one(pool.get_ref())
                    .await
                {
                    let ws_payload = json!({
                        "type": "meeting_left",
                        "meeting_id": meeting_id,
                        "user_id": user_id,
                    });
                    ws::send_to_user(&host_id, ws_payload).await;
                }
                HttpResponse::Ok().body("Successfully left the meeting")
            }
            Err(e) => {
                eprintln!("Error leaving meeting: {:?}", e);
                HttpResponse::InternalServerError().body("Failed to leave meeting")
            }
        }
    } else {
        HttpResponse::Unauthorized().body("Authentication required")
    }
}

// Get Meeting Participants Handler
// Get Meeting Participants Input: GetMeetingParticipantsRequest
// Get Meeting Participants Output: Vec<MeetingParticipant>
pub async fn get_meeting_participants(
    pool: web::Data<PgPool>,
    path: web::Path<Uuid>, // meeting_id passed in URL
) -> impl Responder {
    let meeting_id = path.into_inner();
    let query = "SELECT * FROM meeting_participants WHERE meeting_id = $1";
    match sqlx::query_as::<_, MeetingParticipant>(query)
        .bind(meeting_id)
        .fetch_all(pool.get_ref())
        .await {
            Ok(participants) => HttpResponse::Ok().json(participants),
            Err(e) => {
                eprintln!("Error fetching meeting participants for meeting {}: {:?}", meeting_id, e);
                HttpResponse::InternalServerError().body("Failed to fetch meeting participants")
            }
        }
}

// Start Meeting Handler
// Start Meeting Input: StartMeetingRequest
// Start Meeting Output: GroupMeeting
pub async fn start_meeting(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<Uuid>, // meeting_id in URL
) -> impl Responder {
    // Ensure the request is authenticated.
    if let Some(claims) = req.extensions().get::<Claims>() {
        let meeting_id = path.into_inner();
        
        // Fetch the meeting record.
        let meeting_query = "SELECT * FROM group_meetings WHERE meeting_id = $1";
        let meeting: GroupMeeting = match sqlx::query_as(meeting_query)
            .bind(meeting_id)
            .fetch_one(pool.get_ref())
            .await {
                Ok(m) => m,
                Err(e) => {
                    eprintln!("Error fetching meeting: {:?}", e);
                    return HttpResponse::NotFound().body("Meeting not found");
                }
            };

        // Ensure the requester is the host.
        let host_id = claims.id; // Claims.id is already a Uuid
        if meeting.host_id != host_id {
            return HttpResponse::Forbidden().body("Only the host can start the meeting");
        }

        // Create a new group chat for the meeting.
        let chat_query = "INSERT INTO group_chats (created_at, is_direct) VALUES (NOW(), false) RETURNING group_chat_id, created_at, is_direct";
        let new_chat: GroupChat = match sqlx::query_as(chat_query)
            .fetch_one(pool.get_ref())
            .await {
                Ok(chat) => chat,
                Err(e) => {
                    eprintln!("Error creating meeting chat: {:?}", e);
                    return HttpResponse::InternalServerError().body("Failed to create meeting chat");
                }
            };

        // Update the meeting record: set the new group chat id and mark status as 'ongoing'
        // (Assuming your meeting_status enum has an 'ongoing' variant and the column is defined accordingly.)
        let update_query = "
            UPDATE group_meetings 
            SET group_chat_id = $1, status = 'ongoing'
            WHERE meeting_id = $2
            RETURNING *";
        let updated_meeting: GroupMeeting = match sqlx::query_as(update_query)
            .bind(new_chat.group_chat_id)
            .bind(meeting_id)
            .fetch_one(pool.get_ref())
            .await {
                Ok(m) => m,
                Err(e) => {
                    eprintln!("Error updating meeting: {:?}", e);
                    return HttpResponse::InternalServerError().body("Failed to update meeting");
                }
            };

        // Retrieve all meeting participants.
        let participants_query = "SELECT user_id FROM meeting_participants WHERE meeting_id = $1";
        let participant_ids: Vec<Uuid> = match sqlx::query_scalar(participants_query)
            .bind(meeting_id)
            .fetch_all(pool.get_ref())
            .await {
                Ok(ids) => ids,
                Err(e) => {
                    eprintln!("Error fetching meeting participants: {:?}", e);
                    Vec::new()
                }
            };

        // Build WebSocket payload.
        let ws_payload = json!({
            "type": "meeting_started",
            "meeting_id": meeting_id,
            "group_chat_id": new_chat.group_chat_id
        });

        // Notify all meeting participants via WebSocket.
        for participant in participant_ids {
            ws::send_to_user(&participant, ws_payload.clone()).await;
        }

        HttpResponse::Ok().json(updated_meeting)
    } else {
        HttpResponse::Unauthorized().body("Authentication required")
    }
}

// End Meeting Handler
// End Meeting Input: EndMeetingRequest
// End Meeting Output: GroupMeeting
pub async fn end_meeting(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<Uuid>, // meeting_id from URL
) -> impl Responder {
    if let Some(claims) = req.extensions().get::<Claims>() {
        let meeting_id = path.into_inner();
        let host_id = claims.id; // Claims.id is already a Uuid

        // Fetch the meeting record.
        let meeting_query = "SELECT * FROM group_meetings WHERE meeting_id = $1";
        let meeting: GroupMeeting = match sqlx::query_as::<_, GroupMeeting>(meeting_query)
            .bind(meeting_id)
            .fetch_one(pool.get_ref())
            .await {
                Ok(m) => m,
                Err(e) => {
                    eprintln!("Error fetching meeting: {:?}", e);
                    return HttpResponse::NotFound().body("Meeting not found");
                }
            };

        // Ensure the requester is the host.
        if meeting.host_id != host_id {
            return HttpResponse::Forbidden().body("Only the host can end the meeting");
        }

        // Delete the meeting's group chat if it exists.
        if let Some(gc_id) = meeting.group_chat_id {
            let delete_chat_query = "DELETE FROM group_chats WHERE group_chat_id = $1";
            if let Err(e) = sqlx::query(delete_chat_query)
                .bind(gc_id)
                .execute(pool.get_ref())
                .await {
                    eprintln!("Error deleting meeting chat: {:?}", e);
                    // Optionally, you might choose to proceed even if chat deletion fails.
                }
        }

        // Update the meeting record: set status to 'ended' and clear the group_chat_id.
        let update_query = "
            UPDATE group_meetings 
            SET status = 'ended',
                group_chat_id = NULL
            WHERE meeting_id = $1
            RETURNING *
        ";
        let updated_meeting: GroupMeeting = match sqlx::query_as::<_, GroupMeeting>(update_query)
            .bind(meeting_id)
            .fetch_one(pool.get_ref())
            .await {
                Ok(m) => m,
                Err(e) => {
                    eprintln!("Error updating meeting: {:?}", e);
                    return HttpResponse::InternalServerError().body("Failed to update meeting");
                }
            };

        // Retrieve meeting participants.
        let participants_query = "SELECT user_id FROM meeting_participants WHERE meeting_id = $1";
        let participant_ids: Vec<Uuid> = match sqlx::query_scalar(participants_query)
            .bind(meeting_id)
            .fetch_all(pool.get_ref())
            .await {
                Ok(ids) => ids,
                Err(e) => {
                    eprintln!("Error fetching meeting participants: {:?}", e);
                    Vec::new()
                }
            };

        // Send a WebSocket update to all participants.
        let ws_payload = json!({
            "type": "meeting_ended",
            "meeting_id": meeting_id
        });
        for participant_id in participant_ids {
            ws::send_to_user(&participant_id, ws_payload.clone()).await;
        }

        HttpResponse::Ok().json(updated_meeting)
    } else {
        HttpResponse::Unauthorized().body("Authentication required")
    }
}

// Config Meeting Routes
// POST /meetings/{meeting_id}/join
// DELETE /meetings/{meeting_id}/leave
// GET /meetings/{meeting_id}/participants 
// POST /meetings/{meeting_id}/start
// POST /meetings/{meeting_id}/end
pub fn config_meeting_routes(cfg: &mut web::ServiceConfig) {
    // For creating meetings in a specific support group.
    cfg.service(
        web::scope("/support-groups/{group_id}/meetings")
            .route("", web::post().to(create_support_group_meeting))
            
    );

    // For operations on individual meetings.
    cfg.service(
        web::scope("/meetings")
            .route("/{meeting_id}/join", web::post().to(join_meeting))
            .route("/{meeting_id}/leave", web::delete().to(leave_meeting))
            .route("/{meeting_id}/participants", web::get().to(get_meeting_participants))
            .route("/{meeting_id}/start", web::post().to(start_meeting))
            .route("/{meeting_id}/end", web::post().to(end_meeting))
    );
}
