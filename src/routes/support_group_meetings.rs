use crate::handlers::auth::Claims;

use crate::models::all_models::{
    GroupChat, GroupMeeting, MeetingParticipant, MeetingStatus, SupportGroupStatus,
};
use actix_web::{web, HttpMessage, HttpRequest, HttpResponse, Responder};
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{PgPool, Row};
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
        // Use a transaction to ensure data consistency
        let mut tx = match pool.begin().await {
            Ok(tx) => tx,
            Err(e) => {
                eprintln!("Error starting transaction: {:?}", e);
                return HttpResponse::InternalServerError()
                    .body("Failed to process meeting creation");
            }
        };

        // Ensure the support group exists and is approved, and get its group_chat_id.
        let sg_query = "
            SELECT group_chat_id FROM support_groups 
            WHERE support_group_id = $1 AND status = $2
        ";
        let group_chat_id: Option<Uuid> = match sqlx::query_scalar(sg_query)
            .bind(payload.support_group_id)
            .bind(SupportGroupStatus::Approved)
            .fetch_optional(&mut *tx)
            .await
        {
            Ok(id) => id,
            Err(e) => {
                eprintln!("Error fetching support group: {:?}", e);
                let _ = tx.rollback().await;
                return HttpResponse::InternalServerError().body("Failed to verify support group");
            }
        };

        if group_chat_id.is_none() {
            let _ = tx.rollback().await;
            return HttpResponse::NotFound().body("Support group not found or not approved");
        }

        let query = "
            INSERT INTO group_meetings (meeting_id, group_chat_id, host_id, title, description, scheduled_time, support_group_id, status)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            RETURNING meeting_id, group_chat_id, support_group_id, host_id, title, description, scheduled_time, status, meeting_chat_id
        ";

        let meeting_id = Uuid::new_v4();
        let meeting = match sqlx::query_as::<_, GroupMeeting>(query)
            .bind(meeting_id)
            .bind(group_chat_id)
            .bind(claims.id)
            .bind(&payload.title)
            .bind(&payload.description)
            .bind(&payload.scheduled_time)
            .bind(payload.support_group_id)
            .bind(MeetingStatus::Upcoming)
            .fetch_one(&mut *tx)
            .await
        {
            Ok(m) => m,
            Err(e) => {
                eprintln!("Error creating meeting: {:?}", e);
                let _ = tx.rollback().await;
                return HttpResponse::InternalServerError().body("Failed to create meeting");
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
            return HttpResponse::InternalServerError().body("Failed to add host as participant");
        }

        // Commit the transaction
        if let Err(e) = tx.commit().await {
            eprintln!("Error committing transaction: {:?}", e);
            return HttpResponse::InternalServerError().body("Failed to complete meeting creation");
        }

        HttpResponse::Ok().json(meeting)
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
        let meeting_status = match sqlx::query_scalar::<_, MeetingStatus>(meeting_check)
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

        if meeting_status != MeetingStatus::Upcoming && meeting_status != MeetingStatus::Ongoing {
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

        // If the meeting is ongoing, also add the user to the meeting chat
        if meeting_status == MeetingStatus::Ongoing {
            // Get meeting_chat_id
            let chat_query = "SELECT meeting_chat_id FROM group_meetings WHERE meeting_id = $1";
            let meeting_chat_id: Option<Uuid> = match sqlx::query_scalar(chat_query)
                .bind(payload.meeting_id)
                .fetch_optional(&mut *tx)
                .await
            {
                Ok(id) => id,
                Err(e) => {
                    eprintln!("Error fetching meeting chat ID: {:?}", e);
                    let _ = tx.rollback().await;
                    return HttpResponse::InternalServerError()
                        .body("Failed to fetch meeting chat details");
                }
            };

            // If meeting chat exists, add the user to it
            if let Some(chat_id) = meeting_chat_id {
                let add_to_chat = "
                    INSERT INTO group_chat_members (group_chat_id, user_id)
                    VALUES ($1, $2)
                    ON CONFLICT (group_chat_id, user_id) DO NOTHING
                ";
                if let Err(e) = sqlx::query(add_to_chat)
                    .bind(chat_id)
                    .bind(user_id)
                    .execute(&mut *tx)
                    .await
                {
                    eprintln!("Error adding user to meeting chat: {:?}", e);
                    // Continue even if this fails, as they are at least added as a participant
                }
            }
        }

        // Commit the transaction
        if let Err(e) = tx.commit().await {
            eprintln!("Error committing transaction: {:?}", e);
            return HttpResponse::InternalServerError().body("Failed to complete join process");
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
        let chat_query = "INSERT INTO group_chats (group_chat_id, creator_id, created_at, flagged) VALUES ($1, $2, NOW(), false) RETURNING group_chat_id, created_at, creator_id";
        let chat_id = Uuid::new_v4();
        let new_chat: GroupChat = match sqlx::query_as(chat_query)
            .bind(chat_id) // Specify the UUID for the chat explicitly
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
            SET status = $1, meeting_chat_id = $2
            WHERE meeting_id = $3
            RETURNING meeting_id, group_chat_id, support_group_id, host_id, title, description, scheduled_time, status, meeting_chat_id
        ";
        let updated_meeting = match sqlx::query_as::<_, GroupMeeting>(update_query)
            .bind(MeetingStatus::Ongoing)
            .bind(chat_id)
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
        let mut participant_ids: Vec<Uuid> = match sqlx::query_scalar(participants_query)
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

        // Check if the host is in the participants list
        let host_is_participant = participant_ids.contains(&user_id);

        // If not, add the host as a participant
        if !host_is_participant {
            let add_host_query = "
                INSERT INTO meeting_participants (meeting_id, user_id)
                VALUES ($1, $2)
                ON CONFLICT (meeting_id, user_id) DO NOTHING
            ";
            if let Err(e) = sqlx::query(add_host_query)
                .bind(meeting_id)
                .bind(user_id)
                .execute(&mut *tx)
                .await
            {
                eprintln!("Error adding host as participant: {:?}", e);
                // Continue even if this fails
            }

            // Add host to the participant_ids list for the chat member insertion
            participant_ids.push(user_id);
        }

        // Add all meeting participants to the meeting chat
        for member_id in &participant_ids {
            let add_member_query = "
                INSERT INTO group_chat_members (group_chat_id, user_id)
                VALUES ($1, $2)
                ON CONFLICT (group_chat_id, user_id) DO NOTHING
            ";
            if let Err(e) = sqlx::query(add_member_query)
                .bind(chat_id)
                .bind(member_id)
                .execute(&mut *tx)
                .await
            {
                eprintln!("Error adding member to meeting chat: {:?}", e);
                // Continue with other members even if one fails
            }
        }

        // Commit the transaction
        if let Err(e) = tx.commit().await {
            eprintln!("Error committing transaction: {:?}", e);
            return HttpResponse::InternalServerError()
                .body("Failed to complete meeting start process");
        }

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
            SET status = $1
            WHERE meeting_id = $2
            RETURNING meeting_id, group_chat_id, support_group_id, host_id, title, description, scheduled_time, status, meeting_chat_id
        ";

        let updated_meeting = match sqlx::query_as::<_, GroupMeeting>(update_query)
            .bind(MeetingStatus::Ended)
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

        // Commit the transaction
        if let Err(e) = tx.commit().await {
            eprintln!("Error committing transaction: {:?}", e);
            return HttpResponse::InternalServerError()
                .body("Failed to complete end meeting process");
        }

        HttpResponse::Ok().json(updated_meeting)
    } else {
        HttpResponse::Unauthorized().body("Authentication required")
    }
}

//Get Meeting
//Get Meeting Input: Path (/meetings/{meeting_id})
//Get Meeting Output: GroupMeeting
pub async fn get_meeting(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<Uuid>, // meeting_id passed in URL
) -> impl Responder {
    if let Some(claims) = req.extensions().get::<Claims>() {
        let meeting_id = path.into_inner();
        let user_id = claims.id;

        // Start a transaction
        let mut tx = match pool.begin().await {
            Ok(tx) => tx,
            Err(e) => {
                eprintln!("Error starting transaction: {:?}", e);
                return HttpResponse::InternalServerError().body("Failed to fetch meeting details");
            }
        };

        // Fetch the meeting record
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

        // Check if the user is a participant
        let participant_query =
            "SELECT COUNT(*) FROM meeting_participants WHERE meeting_id = $1 AND user_id = $2";
        let is_participant: i64 = match sqlx::query_scalar(participant_query)
            .bind(meeting_id)
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await
        {
            Ok(count) => count,
            Err(e) => {
                eprintln!("Error checking participant status: {:?}", e);
                0 // Default to not a participant if there's an error
            }
        };

        // Get participant count
        let count_query = "SELECT COUNT(*) FROM meeting_participants WHERE meeting_id = $1";
        let participant_count: i64 = match sqlx::query_scalar(count_query)
            .bind(meeting_id)
            .fetch_one(&mut *tx)
            .await
        {
            Ok(count) => count,
            Err(e) => {
                eprintln!("Error counting participants: {:?}", e);
                0 // Default to 0 if there's an error
            }
        };

        // Commit the transaction
        if let Err(e) = tx.commit().await {
            eprintln!("Error committing transaction: {:?}", e);
            return HttpResponse::InternalServerError().body("Failed to complete meeting fetch");
        }

        // Create a response with additional fields
        let response = json!({
            "data": {
                "meeting_id": meeting.meeting_id,
                "group_chat_id": meeting.group_chat_id,
                "meeting_chat_id": meeting.meeting_chat_id,
                "support_group_id": meeting.support_group_id,
                "host_id": meeting.host_id,
                "title": meeting.title,
                "description": meeting.description,
                "scheduled_time": meeting.scheduled_time,
                "status": meeting.status,
                "participant_count": participant_count,
                "is_participant": is_participant > 0
            }
        });

        HttpResponse::Ok().json(response)
    } else {
        HttpResponse::Unauthorized().body("Authentication required")
    }
}

//Get User Meetings
//Get User Meetings Input: HttpRequest(JWT Token)
//Get User Meetings Output: Vec<GroupMeeting> with additional fields
pub async fn get_user_meetings(pool: web::Data<PgPool>, req: HttpRequest) -> impl Responder {
    if let Some(claims) = req.extensions().get::<Claims>() {
        let user_id = claims.id;

        // Fetch all meetings the user is a participant in
        let query = "
            SELECT gm.*, sg.title as group_title, 
                   COUNT(mp.user_id) as participant_count,
                   true as is_participant,
                   (gm.host_id = $1) as is_host
            FROM group_meetings gm
            JOIN meeting_participants mp ON gm.meeting_id = mp.meeting_id
            JOIN support_groups sg ON gm.support_group_id = sg.support_group_id
            WHERE mp.user_id = $1
            GROUP BY gm.meeting_id, sg.title
            ORDER BY 
                CASE 
                    WHEN gm.status = 'ongoing' THEN 0
                    WHEN gm.status = 'upcoming' THEN 1
                    ELSE 2
                END,
                gm.scheduled_time ASC
        ";

        match sqlx::query(query)
            .bind(user_id)
            .fetch_all(pool.get_ref())
            .await
        {
            Ok(rows) => {
                let meetings: Vec<serde_json::Value> = rows
                    .iter()
                    .map(|row| {
                        let meeting_id: Uuid = row.try_get("meeting_id").unwrap_or_default();
                        let group_chat_id: Option<Uuid> =
                            row.try_get("group_chat_id").unwrap_or(None);
                        let meeting_chat_id: Option<Uuid> =
                            row.try_get("meeting_chat_id").unwrap_or(None);
                        let support_group_id: Uuid =
                            row.try_get("support_group_id").unwrap_or_default();
                        let host_id: Uuid = row.try_get("host_id").unwrap_or_default();
                        let title: String = row.try_get("title").unwrap_or_default();
                        let description: Option<String> =
                            row.try_get("description").unwrap_or(None);
                        let scheduled_time: NaiveDateTime =
                            row.try_get("scheduled_time").unwrap_or_default();
                        let status: MeetingStatus =
                            row.try_get("status").unwrap_or(MeetingStatus::Upcoming);
                        let group_title: String = row.try_get("group_title").unwrap_or_default();
                        let participant_count: i64 = row.try_get("participant_count").unwrap_or(0);
                        let is_participant: bool = row.try_get("is_participant").unwrap_or(false);
                        let is_host: bool = row.try_get("is_host").unwrap_or(false);

                        json!({
                            "meeting_id": meeting_id,
                            "group_chat_id": group_chat_id,
                            "meeting_chat_id": meeting_chat_id,
                            "support_group_id": support_group_id,
                            "host_id": host_id,
                            "title": title,
                            "description": description,
                            "scheduled_time": scheduled_time,
                            "status": status,
                            "group_title": group_title,
                            "participant_count": participant_count,
                            "is_participant": is_participant,
                            "is_host": is_host
                        })
                    })
                    .collect();

                HttpResponse::Ok().json(json!({ "data": meetings }))
            }
            Err(e) => {
                eprintln!("Error fetching user meetings: {:?}", e);
                HttpResponse::InternalServerError().body("Failed to fetch meetings")
            }
        }
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
// GET /meetings/{meeting_id}
// GET /meetings/user
pub fn config_meeting_routes(cfg: &mut web::ServiceConfig) {
    // For operations on individual meetings.
    cfg.service(
        web::scope("/meetings")
            .route("/new", web::post().to(create_support_group_meeting))
            .route("/join", web::post().to(join_meeting))
            .route("/user", web::get().to(get_user_meetings))
            .route("/{meeting_id}/leave", web::delete().to(leave_meeting))
            .route(
                "/{meeting_id}/participants",
                web::get().to(get_meeting_participants),
            )
            .route("/{meeting_id}/start", web::post().to(start_meeting))
            .route("/{meeting_id}/end", web::post().to(end_meeting))
            .route("/{meeting_id}", web::get().to(get_meeting)),
    );
}
