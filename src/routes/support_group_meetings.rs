use crate::handlers::auth::Claims;
use crate::handlers::ws;
use crate::models::all_models::{GroupChat, GroupMeeting, MeetingParticipant};
use actix_web::{web, HttpMessage, HttpRequest, HttpResponse, Responder};
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

//Create Support Group Meeting Request
#[derive(Debug, Deserialize, Serialize)]
pub struct CreateSupportGroupMeetingRequest {
    pub support_group_id: Uuid,
    pub title: String,
    pub description: Option<String>,
    pub scheduled_time: NaiveDateTime,
}

//Create Support Group Meeting
//Create Support Group Meeting Input: HttpRequest(JWT Token), CreateSupportGroupMeetingRequest
//Create Support Group Meeting Output: GroupMeeting
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
        let group_chat_id: Option<Uuid> = match sqlx::query_scalar(sg_query)
            .bind(payload.support_group_id)
            .fetch_optional(pool.get_ref())
            .await
        {
            Ok(id) => id,
            Err(e) => {
                eprintln!("Error fetching support group: {:?}", e);
                return HttpResponse::InternalServerError().body("Failed to verify support group");
            }
        };

        if group_chat_id.is_none() {
            return HttpResponse::NotFound().body("Support group not found or not approved");
        }

        let query = "
            INSERT INTO group_meetings (meeting_id, group_chat_id, host_id, title, description, scheduled_time, support_group_id, status)
            VALUES ($1, $2, $3, $4, $5, $6, $7, 'upcoming')
            RETURNING meeting_id, group_chat_id, support_group_id, host_id, title, description, scheduled_time, status
        ";

        let meeting_id = Uuid::new_v4();
        let meeting = sqlx::query_as::<_, GroupMeeting>(query)
            .bind(meeting_id)
            .bind(group_chat_id)
            .bind(claims.id)
            .bind(&payload.title)
            .bind(&payload.description)
            .bind(&payload.scheduled_time)
            .bind(payload.support_group_id)
            .fetch_one(pool.get_ref())
            .await;

        match meeting {
            Ok(m) => {
                // Use a transaction to ensure data consistency
                let mut tx = match pool.begin().await {
                    Ok(tx) => tx,
                    Err(e) => {
                        eprintln!("Error starting transaction: {:?}", e);
                        return HttpResponse::InternalServerError()
                            .body("Failed to process meeting creation");
                    }
                };

                // Add the host as a participant automatically
                let insert_host = "
                    INSERT INTO meeting_participants (meeting_id, user_id)
                    VALUES ($1, $2)
                ";
                if let Err(e) = sqlx::query(insert_host)
                    .bind(meeting_id)
                    .bind(claims.id)
                    .execute(&mut *tx)
                    .await
                {
                    eprintln!("Error adding host as participant: {:?}", e);
                    let _ = tx.rollback().await;
                    return HttpResponse::InternalServerError()
                        .body("Failed to add host as participant");
                }

                let member_query =
                    "SELECT user_id FROM support_group_members WHERE support_group_id = $1";
                let member_ids: Vec<Uuid> = match sqlx::query_scalar(member_query)
                    .bind(payload.support_group_id)
                    .fetch_all(&mut *tx)
                    .await
                {
                    Ok(ids) => ids,
                    Err(e) => {
                        eprintln!("Error fetching support group members: {:?}", e);
                        let _ = tx.rollback().await;
                        return HttpResponse::InternalServerError()
                            .body("Failed to notify members");
                    }
                };

                // Commit the transaction
                if let Err(e) = tx.commit().await {
                    eprintln!("Error committing transaction: {:?}", e);
                    return HttpResponse::InternalServerError()
                        .body("Failed to complete meeting creation");
                }

                // Send WebSocket notifications
                let ws_payload = json!({
                    "type": "new_meeting",
                    "meeting": m,
                });
                for member_id in member_ids {
                    let _ = ws::send_to_user(&member_id, ws_payload.clone()).await;
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

//Join Meeting Request
#[derive(Debug, Deserialize, Serialize)]
pub struct JoinMeetingRequest {
    pub meeting_id: Uuid,
}

//Join Meeting
//Join Meeting Input: HttpRequest(JWT Token), JoinMeetingRequest
//Join Meeting Output: MeetingParticipant
pub async fn join_meeting(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    payload: web::Json<JoinMeetingRequest>,
) -> impl Responder {
    if let Some(claims) = req.extensions().get::<Claims>() {
        let user_id = claims.id; // Claims.id is already a Uuid

        // Start a transaction
        let mut tx = match pool.begin().await {
            Ok(tx) => tx,
            Err(e) => {
                eprintln!("Error starting transaction: {:?}", e);
                return HttpResponse::InternalServerError().body("Failed to process join request");
            }
        };

        // Check if the meeting exists and is upcoming or ongoing
        let meeting_check = "SELECT status FROM group_meetings WHERE meeting_id = $1";
        let meeting_status = match sqlx::query_scalar::<_, String>(meeting_check)
            .bind(payload.meeting_id)
            .fetch_optional(&mut *tx)
            .await
        {
            Ok(Some(status)) => status,
            Ok(None) => {
                let _ = tx.rollback().await;
                return HttpResponse::NotFound().body("Meeting not found");
            }
            Err(e) => {
                eprintln!("Error checking meeting status: {:?}", e);
                let _ = tx.rollback().await;
                return HttpResponse::InternalServerError().body("Failed to verify meeting");
            }
        };

        if meeting_status != "upcoming" && meeting_status != "ongoing" {
            let _ = tx.rollback().await;
            return HttpResponse::BadRequest().body("Cannot join a meeting that has ended");
        }

        // Check if user is already a participant
        let check_query =
            "SELECT COUNT(*) FROM meeting_participants WHERE meeting_id = $1 AND user_id = $2";
        let count: i64 = match sqlx::query_scalar(check_query)
            .bind(payload.meeting_id)
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await
        {
            Ok(count) => count,
            Err(e) => {
                eprintln!("Error checking existing participation: {:?}", e);
                let _ = tx.rollback().await;
                return HttpResponse::InternalServerError().body("Failed to check participation");
            }
        };

        if count > 0 {
            let _ = tx.rollback().await;
            return HttpResponse::Conflict().body("You are already a participant in this meeting");
        }

        // Insert the participant
        let query = "
            INSERT INTO meeting_participants (meeting_id, user_id)
            VALUES ($1, $2)
            RETURNING meeting_id, user_id
        ";
        let participant = match sqlx::query_as::<_, MeetingParticipant>(query)
            .bind(payload.meeting_id)
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await
        {
            Ok(p) => p,
            Err(e) => {
                eprintln!("Error joining meeting: {:?}", e);
                let _ = tx.rollback().await;
                return HttpResponse::InternalServerError().body("Failed to join meeting");
            }
        };

        // Get the host ID to notify them
        let host_query = "SELECT host_id FROM group_meetings WHERE meeting_id = $1";
        let host_id = match sqlx::query_scalar::<_, Uuid>(host_query)
            .bind(payload.meeting_id)
            .fetch_optional(&mut *tx)
            .await
        {
            Ok(Some(id)) => Some(id),
            Ok(None) => None,
            Err(e) => {
                eprintln!("Error fetching host ID: {:?}", e);
                None
            }
        };

        // Commit the transaction
        if let Err(e) = tx.commit().await {
            eprintln!("Error committing transaction: {:?}", e);
            return HttpResponse::InternalServerError().body("Failed to complete join process");
        }

        // Send WebSocket notification to the host
        if let Some(host_id) = host_id {
            let ws_payload = json!({
                "type": "meeting_joined",
                "meeting_id": payload.meeting_id,
                "user_id": user_id,
            });
            let _ = ws::send_to_user(&host_id, ws_payload).await;
        }

        HttpResponse::Ok().json(participant)
    } else {
        HttpResponse::Unauthorized().body("Authentication required")
    }
}

//Leave Meeting
//Leave Meeting Input: HttpRequest(JWT Token), Path (/meetings/{meeting_id}/leave)
//Leave Meeting Output: Success message
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

        // Start a transaction
        let mut tx = match pool.begin().await {
            Ok(tx) => tx,
            Err(e) => {
                eprintln!("Error starting transaction: {:?}", e);
                return HttpResponse::InternalServerError().body("Failed to process leave request");
            }
        };

        // Check if the user is actually a participant
        let check_query =
            "SELECT COUNT(*) FROM meeting_participants WHERE meeting_id = $1 AND user_id = $2";
        let count: i64 = match sqlx::query_scalar(check_query)
            .bind(meeting_id)
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await
        {
            Ok(count) => count,
            Err(e) => {
                eprintln!("Error checking participation: {:?}", e);
                let _ = tx.rollback().await;
                return HttpResponse::InternalServerError().body("Failed to verify participation");
            }
        };

        if count == 0 {
            let _ = tx.rollback().await;
            return HttpResponse::BadRequest().body("You are not a participant in this meeting");
        }

        // Delete the participant record from the meeting_participants table.
        let delete_query =
            "DELETE FROM meeting_participants WHERE meeting_id = $1 AND user_id = $2";
        match sqlx::query(delete_query)
            .bind(meeting_id)
            .bind(user_id)
            .execute(&mut *tx)
            .await
        {
            Ok(_) => {}
            Err(e) => {
                eprintln!("Error leaving meeting: {:?}", e);
                let _ = tx.rollback().await;
                return HttpResponse::InternalServerError().body("Failed to leave meeting");
            }
        };

        // Get the host ID to notify them
        let host_query = "SELECT host_id FROM group_meetings WHERE meeting_id = $1";
        let host_id = match sqlx::query_scalar::<_, Uuid>(host_query)
            .bind(meeting_id)
            .fetch_optional(&mut *tx)
            .await
        {
            Ok(Some(id)) => Some(id),
            Ok(None) => None,
            Err(e) => {
                eprintln!("Error fetching host ID: {:?}", e);
                None
            }
        };

        // Commit the transaction
        if let Err(e) = tx.commit().await {
            eprintln!("Error committing transaction: {:?}", e);
            return HttpResponse::InternalServerError().body("Failed to complete leave process");
        }

        // Send WebSocket notification to the host
        if let Some(host_id) = host_id {
            let ws_payload = json!({
                "type": "meeting_left",
                "meeting_id": meeting_id,
                "user_id": user_id,
            });
            let _ = ws::send_to_user(&host_id, ws_payload).await;
        }

        HttpResponse::Ok().body("Successfully left the meeting")
    } else {
        HttpResponse::Unauthorized().body("Authentication required")
    }
}

//Get Meeting Participants
//Get Meeting Participants Input: Path (/meetings/{meeting_id}/participants)
//Get Meeting Participants Output: Vec<MeetingParticipant>
pub async fn get_meeting_participants(
    pool: web::Data<PgPool>,
    path: web::Path<Uuid>, // meeting_id passed in URL
) -> impl Responder {
    let meeting_id = path.into_inner();
    let query = "SELECT * FROM meeting_participants WHERE meeting_id = $1";
    match sqlx::query_as::<_, MeetingParticipant>(query)
        .bind(meeting_id)
        .fetch_all(pool.get_ref())
        .await
    {
        Ok(participants) => HttpResponse::Ok().json(participants),
        Err(e) => {
            eprintln!(
                "Error fetching meeting participants for meeting {}: {:?}",
                meeting_id, e
            );
            HttpResponse::InternalServerError().body("Failed to fetch meeting participants")
        }
    }
}

//Start Meeting
//Start Meeting Input: HttpRequest(JWT Token), Path (/meetings/{meeting_id}/start)
//Start Meeting Output: GroupMeeting
pub async fn start_meeting(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<Uuid>, // meeting_id in URL
) -> impl Responder {
    // Ensure the request is authenticated.
    if let Some(claims) = req.extensions().get::<Claims>() {
        let meeting_id = path.into_inner();
        let user_id = claims.id;

        // Start a transaction
        let mut tx = match pool.begin().await {
            Ok(tx) => tx,
            Err(e) => {
                eprintln!("Error starting transaction: {:?}", e);
                return HttpResponse::InternalServerError()
                    .body("Failed to process start meeting request");
            }
        };

        // Fetch the meeting record.
        let meeting_query = "SELECT * FROM group_meetings WHERE meeting_id = $1";
        let meeting: GroupMeeting = match sqlx::query_as(meeting_query)
            .bind(meeting_id)
            .fetch_one(&mut *tx)
            .await
        {
            Ok(m) => m,
            Err(e) => {
                eprintln!("Error fetching meeting: {:?}", e);
                let _ = tx.rollback().await;
                return HttpResponse::NotFound().body("Meeting not found");
            }
        };

        // Ensure the requester is the host.
        if meeting.host_id != user_id {
            let _ = tx.rollback().await;
            return HttpResponse::Forbidden().body("Only the host can start the meeting");
        }

        // Ensure the meeting is in 'upcoming' status
        match meeting.status {
            crate::models::all_models::MeetingStatus::Upcoming => {} // This is what we want
            _ => {
                let _ = tx.rollback().await;
                return HttpResponse::BadRequest().body("Meeting is not in 'upcoming' status");
            }
        }

        // Create a new group chat for the meeting.
        let chat_query = "INSERT INTO group_chats (creator_id, created_at, flagged) VALUES ($1, NOW(), false) RETURNING group_chat_id, created_at, creator_id";
        let new_chat: GroupChat = match sqlx::query_as(chat_query)
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await
        {
            Ok(chat) => chat,
            Err(e) => {
                eprintln!("Error creating meeting chat: {:?}", e);
                let _ = tx.rollback().await;
                return HttpResponse::InternalServerError().body("Failed to create meeting chat");
            }
        };

        // Update the meeting status to 'ongoing' and set the meeting chat.
        let update_query = "
            UPDATE group_meetings 
            SET status = 'ongoing', meeting_chat_id = $1 
            WHERE meeting_id = $2
            RETURNING *
        ";
        let updated_meeting: GroupMeeting = match sqlx::query_as(update_query)
            .bind(new_chat.group_chat_id)
            .bind(meeting_id)
            .fetch_one(&mut *tx)
            .await
        {
            Ok(m) => m,
            Err(e) => {
                eprintln!("Error updating meeting status: {:?}", e);
                let _ = tx.rollback().await;
                return HttpResponse::InternalServerError().body("Failed to update meeting status");
            }
        };

        // Get all participants to add them to the meeting chat and notify them.
        let participants_query = "SELECT user_id FROM meeting_participants WHERE meeting_id = $1";
        let participant_ids: Vec<Uuid> = match sqlx::query_scalar(participants_query)
            .bind(meeting_id)
            .fetch_all(&mut *tx)
            .await
        {
            Ok(ids) => ids,
            Err(e) => {
                eprintln!("Error fetching meeting participants: {:?}", e);
                let _ = tx.rollback().await;
                return HttpResponse::InternalServerError().body("Failed to fetch participants");
            }
        };

        // Add all participants to the meeting chat.
        for participant_id in &participant_ids {
            let add_member_query = "
                INSERT INTO group_chat_members (group_chat_id, user_id)
                VALUES ($1, $2)
            ";
            if let Err(e) = sqlx::query(add_member_query)
                .bind(new_chat.group_chat_id)
                .bind(participant_id)
                .execute(&mut *tx)
                .await
            {
                eprintln!("Error adding participant to meeting chat: {:?}", e);
                // Continue with other participants even if one fails
            }
        }

        // Commit the transaction
        if let Err(e) = tx.commit().await {
            eprintln!("Error committing transaction: {:?}", e);
            return HttpResponse::InternalServerError()
                .body("Failed to complete meeting start process");
        }

        // Notify all participants that the meeting has started.
        let ws_payload = json!({
            "type": "meeting_started",
            "meeting": updated_meeting,
            "meeting_chat_id": new_chat.group_chat_id
        });

        let _ = ws::send_to_users(&participant_ids, ws_payload.clone()).await;

        // Also notify the host
        let _ = ws::send_to_user(&user_id, ws_payload.clone()).await;

        HttpResponse::Ok().json(json!({
            "meeting": updated_meeting,
            "meeting_chat": new_chat
        }))
    } else {
        HttpResponse::Unauthorized().body("Authentication required")
    }
}

//End Meeting
//End Meeting Input: HttpRequest(JWT Token), Path (/meetings/{meeting_id}/end)
//End Meeting Output: GroupMeeting
pub async fn end_meeting(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<Uuid>, // meeting_id from URL
) -> impl Responder {
    if let Some(claims) = req.extensions().get::<Claims>() {
        let meeting_id = path.into_inner();
        let user_id = claims.id;

        // Start a transaction
        let mut tx = match pool.begin().await {
            Ok(tx) => tx,
            Err(e) => {
                eprintln!("Error starting transaction: {:?}", e);
                return HttpResponse::InternalServerError()
                    .body("Failed to process end meeting request");
            }
        };

        // Fetch the meeting record.
        let meeting_query = "SELECT * FROM group_meetings WHERE meeting_id = $1";
        let meeting: GroupMeeting = match sqlx::query_as(meeting_query)
            .bind(meeting_id)
            .fetch_one(&mut *tx)
            .await
        {
            Ok(m) => m,
            Err(e) => {
                eprintln!("Error fetching meeting: {:?}", e);
                let _ = tx.rollback().await;
                return HttpResponse::NotFound().body("Meeting not found");
            }
        };

        // Ensure the requester is the host.
        if meeting.host_id != user_id {
            let _ = tx.rollback().await;
            return HttpResponse::Forbidden().body("Only the host can end the meeting");
        }

        // Ensure the meeting is in 'ongoing' status
        match meeting.status {
            crate::models::all_models::MeetingStatus::Ongoing => {} // This is what we want
            _ => {
                let _ = tx.rollback().await;
                return HttpResponse::BadRequest().body("Meeting is not in 'ongoing' status");
            }
        }

        // Update the meeting status to 'ended'
        let update_query = "
            UPDATE group_meetings 
            SET status = 'ended'
            WHERE meeting_id = $1
            RETURNING *
        ";
        let updated_meeting: GroupMeeting = match sqlx::query_as(update_query)
            .bind(meeting_id)
            .fetch_one(&mut *tx)
            .await
        {
            Ok(m) => m,
            Err(e) => {
                eprintln!("Error updating meeting status: {:?}", e);
                let _ = tx.rollback().await;
                return HttpResponse::InternalServerError().body("Failed to update meeting status");
            }
        };

        // Get all participants to notify them.
        let participants_query = "SELECT user_id FROM meeting_participants WHERE meeting_id = $1";
        let participant_ids: Vec<Uuid> = match sqlx::query_scalar(participants_query)
            .bind(meeting_id)
            .fetch_all(&mut *tx)
            .await
        {
            Ok(ids) => ids,
            Err(e) => {
                eprintln!("Error fetching meeting participants: {:?}", e);
                let _ = tx.rollback().await;
                return HttpResponse::InternalServerError().body("Failed to fetch participants");
            }
        };

        // Commit the transaction
        if let Err(e) = tx.commit().await {
            eprintln!("Error committing transaction: {:?}", e);
            return HttpResponse::InternalServerError()
                .body("Failed to complete meeting end process");
        }

        // Notify all participants that the meeting has ended.
        let ws_payload = json!({
            "type": "meeting_ended",
            "meeting": updated_meeting
        });

        let _ = ws::send_to_users(&participant_ids, ws_payload.clone()).await;

        // Also notify the host
        let _ = ws::send_to_user(&user_id, ws_payload.clone()).await;

        HttpResponse::Ok().json(updated_meeting)
    } else {
        HttpResponse::Unauthorized().body("Authentication required")
    }
}

//Config Meeting Routes
// POST /meetings/create
// POST /meetings/join
// POST /meetings/{meeting_id}/leave
// GET /meetings/{meeting_id}/participants
// POST /meetings/{meeting_id}/start
// POST /meetings/{meeting_id}/end
pub fn config_meeting_routes(cfg: &mut web::ServiceConfig) {
    // For creating meetings in a specific support group.
    cfg.service(
        web::scope("/support-groups/{group_id}/meetings")
            .route("", web::post().to(create_support_group_meeting)),
    );

    // For operations on individual meetings.
    cfg.service(
        web::scope("/meetings")
            .route("/{meeting_id}/join", web::post().to(join_meeting))
            .route("/{meeting_id}/leave", web::delete().to(leave_meeting))
            .route(
                "/{meeting_id}/participants",
                web::get().to(get_meeting_participants),
            )
            .route("/{meeting_id}/start", web::post().to(start_meeting))
            .route("/{meeting_id}/end", web::post().to(end_meeting)),
    );
}
