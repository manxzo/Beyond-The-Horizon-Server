use crate::handlers::auth::Claims;
use crate::handlers::ws; 
use crate::models::all_models::{GroupChat, SupportGroup, SupportGroupMember,GroupMeeting,UserRole};
use actix_web::{HttpMessage, HttpRequest, HttpResponse, Responder, web};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::prelude::FromRow;
use sqlx::PgPool;
use uuid::Uuid;
use chrono::NaiveDateTime;

//Suggest Support Group Request
#[derive(Debug, Deserialize, Serialize)]
pub struct SuggestSupportGroupRequest {
    pub title: String,
    pub description: String,
}

// Suggest Support Group Handler
// Suggest Support Group Input: SuggestSupportGroupRequest
// Suggest Support Group Output: SupportGroup
pub async fn suggest_support_group(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    payload: web::Json<SuggestSupportGroupRequest>,
) -> impl Responder {
    if let Some(_claims) = req.extensions().get::<Claims>() {
        let query = "
            INSERT INTO support_groups (title, description, admin_id, status, created_at)
            VALUES ($1, $2, NULL, 'pending', NOW())
            RETURNING support_group_id, title, description, admin_id, group_chat_id, status, created_at
        ";
        let support_group = sqlx::query_as::<_, SupportGroup>(query)
            .bind(&payload.title)
            .bind(&payload.description)
            .fetch_one(pool.get_ref())
            .await;
        match support_group {
            Ok(sg) => {
                // Optionally, notify admins about the new suggestion.
                let ws_payload = json!({
                    "type": "new_support_group_suggestion",
                    "support_group": sg,
                });
                ws::send_to_role(&UserRole::Admin, ws_payload).await;
                HttpResponse::Ok().json(sg)
            }
            Err(e) => {
                eprintln!("Error suggesting support group: {:?}", e);
                HttpResponse::InternalServerError().body("Failed to suggest support group")
            }
        }
    } else {
        HttpResponse::Unauthorized().body("Authentication required")
    }
}

// Support Group Summary
#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct SupportGroupSummary {
    pub support_group_id: Uuid,
    pub title: String,
    pub description: String,
    pub created_at: NaiveDateTime,
    pub member_count: i64,
}

// List Support Groups Handler
// List Support Groups Input: JWT Token
// List Support Groups Output: Vec<SupportGroupSummary>
pub async fn list_support_groups(
    pool: web::Data<PgPool>,
    req: HttpRequest,
) -> impl Responder {
    // Ensure the request is authenticated.
    if req.extensions().get::<Claims>().is_none() {
        return HttpResponse::Unauthorized().body("Authentication required");
    }

    // Query to get support groups and count their members.
    let query = r#"
        SELECT 
            sg.support_group_id, 
            sg.title, 
            sg.description, 
            sg.created_at,
            COUNT(sgm.user_id) AS member_count
        FROM support_groups sg
        LEFT JOIN support_group_members sgm 
            ON sg.support_group_id = sgm.support_group_id
        WHERE sg.status = 'approved'
        GROUP BY sg.support_group_id, sg.title, sg.description, sg.created_at
        ORDER BY sg.created_at DESC
    "#;

    match sqlx::query_as::<_, SupportGroupSummary>(query)
        .fetch_all(pool.get_ref())
        .await
    {
        Ok(groups) => HttpResponse::Ok().json(groups),
        Err(e) => {
            eprintln!("Error listing support groups: {:?}", e);
            HttpResponse::InternalServerError().body("Failed to list support groups")
        }
    }
}
// Join Support Group Request
#[derive(Debug, Deserialize, Serialize)]
pub struct JoinSupportGroupRequest {
    pub support_group_id: Uuid,
}

// Join Support Group Handler
// Join Support Group Input: JoinSupportGroupRequest
// Join Support Group Output: SupportGroupMember
pub async fn join_support_group(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    payload: web::Json<JoinSupportGroupRequest>,
) -> impl Responder {
    if let Some(claims) = req.extensions().get::<Claims>() {
        let user_id = claims.id;

        let query = "
            INSERT INTO support_group_members (support_group_id, user_id, joined_at)
            VALUES ($1, $2, NOW())
            RETURNING support_group_id, user_id, joined_at
        ";
        let membership = sqlx::query_as::<_, SupportGroupMember>(query)
            .bind(payload.support_group_id)
            .bind(user_id)
            .fetch_one(pool.get_ref())
            .await;
        match membership {
            Ok(m) => {
                // Get all current members of the group to notify them
                let members_query = "
                    SELECT user_id FROM support_group_members 
                    WHERE support_group_id = $1 AND user_id != $2
                ";
                let members = sqlx::query_scalar::<_, Uuid>(members_query)
                    .bind(payload.support_group_id)
                    .bind(&user_id)
                    .fetch_all(pool.get_ref())
                    .await;
                
                if let Ok(members) = members {
                    // Notify only the members of this specific support group
                    let ws_payload = json!({
                        "type": "joined_support_group",
                        "support_group_id": payload.support_group_id,
                        "user_id": user_id
                    });
                    
                    ws::send_to_users(&members, ws_payload).await;
                }
                
                HttpResponse::Ok().json(m)
            }
            Err(e) => {
                eprintln!("Error joining support group: {:?}", e);
                HttpResponse::InternalServerError().body("Failed to join support group")
            }
        }
    } else {
        HttpResponse::Unauthorized().body("Authentication required")
    }
}


// Leave Support Group Handler
// Leave Support Group Input: SupportGroupId
// Leave Support Group Output: String
pub async fn leave_support_group(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<Uuid>,
) -> impl Responder {
    // Check that the request is authenticated.
    if let Some(claims) = req.extensions().get::<Claims>() {
        let user_id = &claims.id;
        let support_group_id = path.into_inner();

        // Get all current members of the group before removing the user
        let members_query = "
            SELECT user_id FROM support_group_members 
            WHERE support_group_id = $1 AND user_id != $2
        ";
        let members = sqlx::query_scalar::<_, Uuid>(members_query)
            .bind(support_group_id)
            .bind(user_id)
            .fetch_all(pool.get_ref())
            .await;

        // Delete the membership record from support_group_members.
        let result = sqlx::query("DELETE FROM support_group_members WHERE support_group_id = $1 AND user_id = $2")
            .bind(support_group_id)
            .bind(user_id)
            .execute(pool.get_ref())
            .await;

        match result {
            Ok(_) => {
                // Send a WebSocket update to notify only the members of this specific group
                if let Ok(members) = members {
                    let ws_payload = json!({
                        "type": "left_support_group",
                        "support_group_id": support_group_id,
                        "user_id": user_id,
                    });
                    
                    ws::send_to_users(&members, ws_payload).await;
                }

                HttpResponse::Ok().body("Successfully left the support group")
            }
            Err(e) => {
                eprintln!("Error leaving support group: {:?}", e);
                HttpResponse::InternalServerError().body("Failed to leave support group")
            }
        }
    } else {
        HttpResponse::Unauthorized().body("Authentication required")
    }
}

// Sponsor Info
#[derive(Debug, Serialize,Deserialize,FromRow)]
pub struct SponsorInfo {
    pub user_id: Uuid,
    pub username: String,
    pub avatar_url: String,
    pub role: String,
}

// Support Group Details
#[derive(Debug, Serialize,Deserialize)]
pub struct SupportGroupDetails {
    pub group: SupportGroup,
    pub members: Vec<SupportGroupMember>,
    pub sponsors: Vec<SponsorInfo>,
    pub main_group_chat: Option<GroupChat>,
    pub meetings: Vec<GroupMeeting>,
    pub meeting_group_chats: Vec<GroupChat>,
}

// Get Support Group Details Handler
// Get Support Group Details Input: SupportGroupId
// Get Support Group Details Output: SupportGroupDetails
pub async fn get_support_group_details(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<Uuid>,
) -> impl Responder {
    // Check authentication.
    if req.extensions().get::<Claims>().is_none() {
        return HttpResponse::Unauthorized().body("Authentication required");
    }

    let support_group_id = path.into_inner();

    // Retrieve the support group record.
    let group_query = "SELECT * FROM support_groups WHERE support_group_id = $1";
    let group: SupportGroup = match sqlx::query_as::<_, SupportGroup>(group_query)
        .bind(support_group_id)
        .fetch_one(pool.get_ref())
        .await {
            Ok(g) => g,
            Err(e) => {
                eprintln!("Error fetching support group: {:?}", e);
                return HttpResponse::NotFound().body("Support group not found");
            }
        };

    // Retrieve all members of the support group.
    let members_query = "SELECT * FROM support_group_members WHERE support_group_id = $1";
    let members: Vec<SupportGroupMember> = match sqlx::query_as::<_, SupportGroupMember>(members_query)
        .bind(support_group_id)
        .fetch_all(pool.get_ref())
        .await {
            Ok(m) => m,
            Err(e) => {
                eprintln!("Error fetching group members: {:?}", e);
                Vec::new()
            }
        };

    // Retrieve sponsors by joining support_group_members with users filtering for role 'sponsor'
    let sponsors_query = r#"
        SELECT u.user_id, u.username, u.avatar_url, u.role
        FROM support_group_members sgm
        JOIN users u ON sgm.user_id = u.user_id
        WHERE sgm.support_group_id = $1 AND u.role = 'sponsor'
    "#;
    let sponsors: Vec<SponsorInfo> = match sqlx::query_as::<_, SponsorInfo>(sponsors_query)
        .bind(support_group_id)
        .fetch_all(pool.get_ref())
        .await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Error fetching sponsors: {:?}", e);
                Vec::new()
            }
        };

    // Retrieve the main group chat for the support group.
    let main_group_chat = if let Some(gc_id) = group.group_chat_id {
        let chat_query = "SELECT * FROM group_chats WHERE group_chat_id = $1";
        match sqlx::query_as::<_, GroupChat>(chat_query)
            .bind(gc_id)
            .fetch_one(pool.get_ref())
            .await {
                Ok(chat) => Some(chat),
                Err(e) => {
                    eprintln!("Error fetching main group chat: {:?}", e);
                    None
                }
            }
    } else {
        None
    };

    // Retrieve all meetings associated with the support group.
    let meetings_query = "SELECT * FROM group_meetings WHERE support_group_id = $1 ORDER BY scheduled_time ASC";
    let meetings: Vec<GroupMeeting> = match sqlx::query_as::<_, GroupMeeting>(meetings_query)
        .bind(support_group_id)
        .fetch_all(pool.get_ref())
        .await {
            Ok(ms) => ms,
            Err(e) => {
                eprintln!("Error fetching meetings: {:?}", e);
                Vec::new()
            }
        };

    // Retrieve all distinct group chats associated with these meetings.
    let meeting_group_chats: Vec<GroupChat> = match sqlx::query_as::<_, GroupChat>(
        "SELECT DISTINCT gc.* FROM group_meetings gm \
         JOIN group_chats gc ON gm.group_chat_id = gc.group_chat_id \
         WHERE gm.support_group_id = $1"
    )
        .bind(support_group_id)
        .fetch_all(pool.get_ref())
        .await {
            Ok(chats) => chats,
            Err(e) => {
                eprintln!("Error fetching meeting group chats: {:?}", e);
                Vec::new()
            }
        };

    let details = SupportGroupDetails {
        group,
        members,
        sponsors,
        main_group_chat,
        meetings,
        meeting_group_chats,
    };

    HttpResponse::Ok().json(details)
}

// User Support Groups
#[derive(Debug, Serialize,Deserialize,FromRow)]
pub struct UserSupportGroups {
    pub support_group_id: Uuid,
    pub title: String,
    pub description: String,
    pub joined_at: chrono::NaiveDateTime,
}

// List My Support Groups Handler
// List My Support Groups Input: JWT Token
// List My Support Groups Output: Vec<UserSupportGroups>
pub async fn list_my_support_groups(
    pool: web::Data<PgPool>,
    req: actix_web::HttpRequest,
) -> impl Responder {
    if let Some(claims) = req.extensions().get::<Claims>() {
        let user_id = &claims.id;

        let query = r#"
            SELECT sg.support_group_id, sg.title, sg.description, sgm.joined_at
            FROM support_groups sg
            JOIN support_group_members sgm ON sg.support_group_id = sgm.support_group_id
            WHERE sgm.user_id = $1 AND sg.status = 'approved'
            ORDER BY sgm.joined_at DESC
        "#;

        match sqlx::query_as::<_, UserSupportGroups>(query)
            .bind(user_id)
            .fetch_all(pool.get_ref())
            .await {
                Ok(groups) => HttpResponse::Ok().json(groups),
                Err(e) => {
                    eprintln!("Error listing my support groups: {:?}", e);
                    HttpResponse::InternalServerError().body("Failed to list support groups")
                }
            }
    } else {
        HttpResponse::Unauthorized().body("Authentication required")
    }
}

// Config Support Group Routes
// POST /support-groups/suggest
// GET /support-groups/list
// GET /support-groups/{group_id}
// POST /support-groups/join
// DELETE /support-groups/leave
// GET /support-groups/my
pub fn config_support_group_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/support-groups")
            .route("/suggest", web::post().to(suggest_support_group))
            .route("/list", web::get().to(list_support_groups))
            .route("/{group_id}", web::get().to(get_support_group_details))
            .route("/join", web::post().to(join_support_group))
            .route("/{group_id}/leave", web::delete().to(leave_support_group))
            .route("/my", web::get().to(list_my_support_groups))
    );
}

