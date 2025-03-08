use crate::handlers::auth::Claims;
use crate::handlers::ws;
use crate::models::all_models::{ApplicationStatus, ReportedType, UserRole};
use actix_web::{HttpMessage, HttpRequest, HttpResponse, Responder, web};
use chrono::{NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{PgPool, Row};
use uuid::Uuid;

//Admin Action Response
#[derive(Debug, Deserialize, Serialize)]
pub struct AdminActionResponse {
    pub success: bool,
    pub message: String,
}

//Review Sponsor Application Request
#[derive(Debug, Deserialize, Serialize)]
pub struct ReviewSponsorApplicationRequest {
    pub application_id: Uuid,
    pub status: ApplicationStatus,
    pub admin_comments: Option<String>,
}

//Review Support Group Request
#[derive(Debug, Deserialize, Serialize)]
pub struct ReviewSupportGroupRequest {
    pub support_group_id: Uuid,
    pub status: ApplicationStatus,
    pub admin_comments: Option<String>,
}

//Review Resource Request
#[derive(Debug, Deserialize, Serialize)]
pub struct ReviewResourceRequest {
    pub resource_id: Uuid,
    pub approved: bool,
    pub admin_comments: Option<String>,
}

//Handle Report Request
#[derive(Debug, Deserialize, Serialize)]
pub struct HandleReportRequest {
    pub report_id: Uuid,
    pub action_taken: String,
    pub resolved: bool,
}

//Ban User Request
#[derive(Debug, Deserialize, Serialize)]
pub struct BanUserRequest {
    pub user_id: Uuid,
    pub ban_duration_days: Option<i32>, // None means permanent ban
    pub reason: String,
}

//Unban User Request
#[derive(Debug, Deserialize, Serialize)]
pub struct UnbanUserRequest {
    pub user_id: Uuid,
}

//Get Admin Stats Response
#[derive(Debug, Serialize)]
pub struct GetAdminStatsResponse {
    pub total_users: i64,
    pub total_sponsors: i64,
    pub pending_sponsor_applications: i64,
    pub pending_support_groups: i64,
    pub pending_resources: i64,
    pub unresolved_reports: i64,
}

//Ensure Admin Helper Function
async fn ensure_admin(req: &HttpRequest) -> Result<(), HttpResponse> {
    if let Some(claims) = req.extensions().get::<Claims>() {
        if claims.role == UserRole::Admin {
            Ok(())
        } else {
            Err(HttpResponse::Forbidden().body("Admin access required"))
        }
    } else {
        Err(HttpResponse::Unauthorized().body("Authentication required"))
    }
}

//Get Pending Sponsor Applications
//Get Pending Sponsor Applications Input: HttpRequest(JWT Token)
//Get Pending Sponsor Applications Output: Vec<SponsorApplication>
pub async fn get_pending_sponsor_applications(
    pool: web::Data<PgPool>,
    req: HttpRequest,
) -> impl Responder {
    // Check if user is admin
    if let Err(response) = ensure_admin(&req).await {
        return response;
    }

    // Get all pending sponsor applications
    let query = r#"
        SELECT 
            sa.application_id, 
            sa.user_id, 
            sa.status, 
            sa.application_info, 
            sa.reviewed_by, 
            sa.admin_comments, 
            sa.created_at,
            u.username,
            u.email
        FROM 
            sponsor_applications sa
        JOIN 
            users u ON sa.user_id = u.user_id
        WHERE 
            sa.status = 'pending'
        ORDER BY 
            sa.created_at DESC
    "#;

    match sqlx::query(query).fetch_all(pool.get_ref()).await {
        Ok(rows) => {
            let applications = rows
                .iter()
                .map(|row| {
                    json!({
                        "application_id": row.get::<Uuid, _>("application_id"),
                        "user_id": row.get::<Uuid, _>("user_id"),
                        "username": row.get::<String, _>("username"),
                        "email": row.get::<String, _>("email"),
                        "status": row.get::<ApplicationStatus, _>("status"),
                        "application_info": row.get::<serde_json::Value, _>("application_info"),
                        "reviewed_by": row.get::<Option<Uuid>, _>("reviewed_by"),
                        "admin_comments": row.get::<Option<String>, _>("admin_comments"),
                        "created_at": row.get::<NaiveDateTime, _>("created_at"),
                    })
                })
                .collect::<Vec<_>>();

            HttpResponse::Ok().json(applications)
        }
        Err(e) => {
            eprintln!("Database error: {:?}", e);
            HttpResponse::InternalServerError().body("Failed to fetch sponsor applications")
        }
    }
}

//Review Sponsor Application
//Review Sponsor Application Input: HttpRequest(JWT Token), ReviewSponsorApplicationRequest
//Review Sponsor Application Output: AdminActionResponse
pub async fn review_sponsor_application(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    payload: web::Json<ReviewSponsorApplicationRequest>,
) -> impl Responder {
    // Check if user is admin
    if let Err(response) = ensure_admin(&req).await {
        return response;
    }

    // Get admin ID from claims
    let admin_id = if let Some(claims) = req.extensions().get::<Claims>() {
        claims.id
    } else {
        return HttpResponse::Unauthorized().body("Authentication required");
    };

    // Validate input
    if payload.status != ApplicationStatus::Approved
        && payload.status != ApplicationStatus::Rejected
    {
        return HttpResponse::BadRequest().body("Invalid status. Must be 'approved' or 'rejected'");
    }

    // Start a transaction
    let mut tx = match pool.begin().await {
        Ok(tx) => tx,
        Err(e) => {
            eprintln!("Failed to start transaction: {:?}", e);
            return HttpResponse::InternalServerError().body("Database error");
        }
    };

    // Update the application status
    let update_query = r#"
        UPDATE sponsor_applications
        SET 
            status = $1, 
            reviewed_by = $2, 
            admin_comments = $3
        WHERE 
            application_id = $4
        RETURNING user_id
    "#;

    let user_id = match sqlx::query_scalar::<_, Uuid>(update_query)
        .bind(&payload.status)
        .bind(admin_id)
        .bind(&payload.admin_comments)
        .bind(payload.application_id)
        .fetch_one(&mut *tx)
        .await
    {
        Ok(user_id) => user_id,
        Err(e) => {
            eprintln!("Failed to update application: {:?}", e);
            let _ = tx.rollback().await;
            return HttpResponse::InternalServerError().body("Failed to update application");
        }
    };

    // If approved, update the user's role to Sponsor
    if payload.status == ApplicationStatus::Approved {
        let update_user_query = r#"
            UPDATE users
            SET role = 'sponsor'
            WHERE user_id = $1
        "#;

        if let Err(e) = sqlx::query(update_user_query)
            .bind(user_id)
            .execute(&mut *tx)
            .await
        {
            eprintln!("Failed to update user role: {:?}", e);
            let _ = tx.rollback().await;
            return HttpResponse::InternalServerError().body("Failed to update user role");
        }
    }

    // Commit the transaction
    if let Err(e) = tx.commit().await {
        eprintln!("Failed to commit transaction: {:?}", e);
        return HttpResponse::InternalServerError().body("Database error");
    }

    // Send notification to the user
    let notification = json!({
        "type": "sponsor_application_reviewed",
        "data": {
            "application_id": payload.application_id,
            "status": payload.status,
            "admin_comments": payload.admin_comments
        }
    });

    // Send WebSocket notification to the user
    ws::send_to_user(&user_id, notification.clone()).await;

    // Return success response
    HttpResponse::Ok().json(AdminActionResponse {
        success: true,
        message: format!("Sponsor application {} successfully", payload.status),
    })
}

//Get Pending Support Groups
//Get Pending Support Groups Input: HttpRequest(JWT Token)
//Get Pending Support Groups Output: Vec<SupportGroup>
pub async fn get_pending_support_groups(
    pool: web::Data<PgPool>,
    req: HttpRequest,
) -> impl Responder {
    // Check if user is admin
    if let Err(response) = ensure_admin(&req).await {
        return response;
    }

    // Get all pending support groups
    let query = r#"
        SELECT 
            sg.support_group_id, 
            sg.title, 
            sg.description, 
            sg.admin_id, 
            sg.group_chat_id, 
            sg.status, 
            sg.created_at,
            u.username,
            u.email
        FROM 
            support_groups sg
        JOIN 
            users u ON sg.admin_id = u.user_id
        WHERE 
            sg.status = 'pending'
        ORDER BY 
            sg.created_at DESC
    "#;

    match sqlx::query(query).fetch_all(pool.get_ref()).await {
        Ok(rows) => {
            let support_groups = rows
                .iter()
                .map(|row| {
                    json!({
                        "support_group_id": row.get::<Uuid, _>("support_group_id"),
                        "title": row.get::<String, _>("title"),
                        "description": row.get::<String, _>("description"),
                        "admin_id": row.get::<Uuid, _>("admin_id"),
                        "admin_username": row.get::<String, _>("username"),
                        "admin_email": row.get::<String, _>("email"),
                        "group_chat_id": row.get::<Option<Uuid>, _>("group_chat_id"),
                        "status": row.get::<ApplicationStatus, _>("status"),
                        "created_at": row.get::<NaiveDateTime, _>("created_at"),
                    })
                })
                .collect::<Vec<_>>();

            HttpResponse::Ok().json(support_groups)
        }
        Err(e) => {
            eprintln!("Database error: {:?}", e);
            HttpResponse::InternalServerError().body("Failed to fetch support groups")
        }
    }
}

//Review Support Group
//Review Support Group Input: HttpRequest(JWT Token), ReviewSupportGroupRequest
//Review Support Group Output: AdminActionResponse
pub async fn review_support_group(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    payload: web::Json<ReviewSupportGroupRequest>,
) -> impl Responder {
    // Check if user is admin
    if let Err(response) = ensure_admin(&req).await {
        return response;
    }

    // Get admin ID from claims
    let admin_id = if let Some(claims) = req.extensions().get::<Claims>() {
        claims.id
    } else {
        return HttpResponse::Unauthorized().body("Authentication required");
    };

    // Validate input
    if payload.status != ApplicationStatus::Approved
        && payload.status != ApplicationStatus::Rejected
    {
        return HttpResponse::BadRequest().body("Invalid status. Must be 'approved' or 'rejected'");
    }

    // Start a transaction
    let mut tx = match pool.begin().await {
        Ok(tx) => tx,
        Err(e) => {
            eprintln!("Failed to start transaction: {:?}", e);
            return HttpResponse::InternalServerError().body("Database error");
        }
    };

    // Get the support group admin ID
    let get_admin_query = r#"
        SELECT admin_id
        FROM support_groups
        WHERE support_group_id = $1
    "#;

    let group_admin_id = match sqlx::query_scalar::<_, Uuid>(get_admin_query)
        .bind(payload.support_group_id)
        .fetch_optional(&mut *tx)
        .await
    {
        Ok(Some(admin_id)) => admin_id,
        Ok(None) => {
            let _ = tx.rollback().await;
            return HttpResponse::NotFound().body("Support group not found");
        }
        Err(e) => {
            eprintln!("Failed to get support group admin: {:?}", e);
            let _ = tx.rollback().await;
            return HttpResponse::InternalServerError().body("Database error");
        }
    };

    // If approved, create a group chat for the support group if it doesn't exist
    let mut group_chat_id = None;
    if payload.status == ApplicationStatus::Approved {
        // Check if group chat already exists
        let check_chat_query = r#"
            SELECT group_chat_id
            FROM support_groups
            WHERE support_group_id = $1
        "#;

        match sqlx::query_scalar::<_, Option<Uuid>>(check_chat_query)
            .bind(payload.support_group_id)
            .fetch_one(&mut *tx)
            .await
        {
            Ok(Some(chat_id)) => {
                group_chat_id = Some(chat_id);
            }
            Ok(None) => {
                // Create a new group chat
                let new_chat_id = Uuid::new_v4();
                let create_chat_query = r#"
                    INSERT INTO group_chats (group_chat_id, name, created_by, created_at)
                    VALUES ($1, $2, $3, $4)
                "#;

                // Get support group title
                let get_title_query = r#"
                    SELECT title
                    FROM support_groups
                    WHERE support_group_id = $1
                "#;

                let title = match sqlx::query_scalar::<_, String>(get_title_query)
                    .bind(payload.support_group_id)
                    .fetch_one(&mut *tx)
                    .await
                {
                    Ok(title) => title,
                    Err(e) => {
                        eprintln!("Failed to get support group title: {:?}", e);
                        let _ = tx.rollback().await;
                        return HttpResponse::InternalServerError().body("Database error");
                    }
                };

                let chat_name = format!("Support Group: {}", title);

                if let Err(e) = sqlx::query(create_chat_query)
                    .bind(new_chat_id)
                    .bind(&chat_name)
                    .bind(admin_id)
                    .bind(Utc::now().naive_utc())
                    .execute(&mut *tx)
                    .await
                {
                    eprintln!("Failed to create group chat: {:?}", e);
                    let _ = tx.rollback().await;
                    return HttpResponse::InternalServerError().body("Failed to create group chat");
                }

                // Add the support group admin as a member of the group chat
                let add_member_query = r#"
                    INSERT INTO group_chat_members (group_chat_id, user_id, joined_at)
                    VALUES ($1, $2, $3)
                "#;

                if let Err(e) = sqlx::query(add_member_query)
                    .bind(new_chat_id)
                    .bind(group_admin_id)
                    .bind(Utc::now().naive_utc())
                    .execute(&mut *tx)
                    .await
                {
                    eprintln!("Failed to add admin to group chat: {:?}", e);
                    let _ = tx.rollback().await;
                    return HttpResponse::InternalServerError()
                        .body("Failed to add admin to group chat");
                }

                group_chat_id = Some(new_chat_id);
            }
            Err(e) => {
                eprintln!("Failed to check for existing group chat: {:?}", e);
                let _ = tx.rollback().await;
                return HttpResponse::InternalServerError().body("Database error");
            }
        }
    }

    // Update the support group status
    let update_query = r#"
        UPDATE support_groups
        SET 
            status = $1,
            group_chat_id = $2
        WHERE 
            support_group_id = $3
    "#;

    if let Err(e) = sqlx::query(update_query)
        .bind(&payload.status)
        .bind(group_chat_id)
        .bind(payload.support_group_id)
        .execute(&mut *tx)
        .await
    {
        eprintln!("Failed to update support group: {:?}", e);
        let _ = tx.rollback().await;
        return HttpResponse::InternalServerError().body("Failed to update support group");
    }

    // Commit the transaction
    if let Err(e) = tx.commit().await {
        eprintln!("Failed to commit transaction: {:?}", e);
        return HttpResponse::InternalServerError().body("Database error");
    }

    // Send notification to the support group admin
    let notification = json!({
        "type": "support_group_reviewed",
        "data": {
            "support_group_id": payload.support_group_id,
            "status": payload.status,
            "admin_comments": payload.admin_comments,
            "group_chat_id": group_chat_id
        }
    });

    // Send WebSocket notification to the user
    ws::send_to_user(&group_admin_id, notification.clone()).await;

    // Return success response
    HttpResponse::Ok().json(AdminActionResponse {
        success: true,
        message: format!("Support group {} successfully", payload.status),
    })
}

//Get Pending Resources
//Get Pending Resources Input: HttpRequest(JWT Token)
//Get Pending Resources Output: Vec<Resource>
pub async fn get_pending_resources(pool: web::Data<PgPool>, req: HttpRequest) -> impl Responder {
    // Check if user is admin
    if let Err(response) = ensure_admin(&req).await {
        return response;
    }

    // Get all pending resources
    let query = r#"
        SELECT 
            r.resource_id, 
            r.contributor_id, 
            r.title, 
            r.content, 
            r.approved, 
            r.created_at, 
            r.support_group_id,
            u.username,
            u.email
        FROM 
            resources r
        JOIN 
            users u ON r.contributor_id = u.user_id
        WHERE 
            r.approved = false
        ORDER BY 
            r.created_at DESC
    "#;

    match sqlx::query(query).fetch_all(pool.get_ref()).await {
        Ok(rows) => {
            let resources = rows
                .iter()
                .map(|row| {
                    json!({
                        "resource_id": row.get::<Uuid, _>("resource_id"),
                        "contributor_id": row.get::<Uuid, _>("contributor_id"),
                        "contributor_username": row.get::<String, _>("username"),
                        "contributor_email": row.get::<String, _>("email"),
                        "title": row.get::<String, _>("title"),
                        "content": row.get::<String, _>("content"),
                        "approved": row.get::<bool, _>("approved"),
                        "created_at": row.get::<NaiveDateTime, _>("created_at"),
                        "support_group_id": row.get::<Option<Uuid>, _>("support_group_id"),
                    })
                })
                .collect::<Vec<_>>();

            HttpResponse::Ok().json(resources)
        }
        Err(e) => {
            eprintln!("Database error: {:?}", e);
            HttpResponse::InternalServerError().body("Failed to fetch resources")
        }
    }
}

//Review Resource
//Review Resource Input: HttpRequest(JWT Token), ReviewResourceRequest
//Review Resource Output: AdminActionResponse
pub async fn review_resource(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    payload: web::Json<ReviewResourceRequest>,
) -> impl Responder {
    // Check if user is admin
    if let Err(response) = ensure_admin(&req).await {
        return response;
    }

    // Get admin ID from claims
    let admin_id = if let Some(claims) = req.extensions().get::<Claims>() {
        claims.id
    } else {
        return HttpResponse::Unauthorized().body("Authentication required");
    };

    // Start a transaction
    let mut tx = match pool.begin().await {
        Ok(tx) => tx,
        Err(e) => {
            eprintln!("Failed to start transaction: {:?}", e);
            return HttpResponse::InternalServerError().body("Database error");
        }
    };

    // Get the resource contributor ID
    let get_contributor_query = r#"
        SELECT contributor_id
        FROM resources
        WHERE resource_id = $1
    "#;

    let contributor_id = match sqlx::query_scalar::<_, Uuid>(get_contributor_query)
        .bind(payload.resource_id)
        .fetch_optional(&mut *tx)
        .await
    {
        Ok(Some(contributor_id)) => contributor_id,
        Ok(None) => {
            let _ = tx.rollback().await;
            return HttpResponse::NotFound().body("Resource not found");
        }
        Err(e) => {
            eprintln!("Failed to get resource contributor: {:?}", e);
            let _ = tx.rollback().await;
            return HttpResponse::InternalServerError().body("Database error");
        }
    };

    // Update the resource approval status
    let update_query = r#"
        UPDATE resources
        SET approved = $1
        WHERE resource_id = $2
        RETURNING title
    "#;

    let resource_title = match sqlx::query_scalar::<_, String>(update_query)
        .bind(payload.approved)
        .bind(payload.resource_id)
        .fetch_one(&mut *tx)
        .await
    {
        Ok(title) => title,
        Err(e) => {
            eprintln!("Failed to update resource: {:?}", e);
            let _ = tx.rollback().await;
            return HttpResponse::InternalServerError().body("Failed to update resource");
        }
    };

    // If admin comments are provided, store them in a separate table
    if let Some(comments) = &payload.admin_comments {
        let comments_query = r#"
            INSERT INTO admin_comments (resource_id, admin_id, comments, created_at)
            VALUES ($1, $2, $3, $4)
        "#;

        if let Err(e) = sqlx::query(comments_query)
            .bind(payload.resource_id)
            .bind(admin_id)
            .bind(comments)
            .bind(Utc::now().naive_utc())
            .execute(&mut *tx)
            .await
        {
            eprintln!("Failed to store admin comments: {:?}", e);
            // Continue even if comments storage fails
        }
    }

    // Commit the transaction
    if let Err(e) = tx.commit().await {
        eprintln!("Failed to commit transaction: {:?}", e);
        return HttpResponse::InternalServerError().body("Database error");
    }

    // Send notification to the resource contributor
    let notification = json!({
        "type": "resource_reviewed",
        "data": {
            "resource_id": payload.resource_id,
            "resource_title": resource_title,
            "approved": payload.approved,
            "admin_comments": payload.admin_comments
        }
    });

    // Send WebSocket notification to the user
    ws::send_to_user(&contributor_id, notification.clone()).await;

    // Return success response
    let status_text = if payload.approved {
        "approved"
    } else {
        "rejected"
    };
    HttpResponse::Ok().json(AdminActionResponse {
        success: true,
        message: format!("Resource {} successfully", status_text),
    })
}

//Get Unresolved Reports
//Get Unresolved Reports Input: HttpRequest(JWT Token)
//Get Unresolved Reports Output: Vec<Report>
pub async fn get_unresolved_reports(pool: web::Data<PgPool>, req: HttpRequest) -> impl Responder {
    // Check if user is admin
    if let Err(response) = ensure_admin(&req).await {
        return response;
    }

    // Get all unresolved reports
    let query = r#"
        SELECT 
            r.report_id, 
            r.reporter_id, 
            r.reported_user_id, 
            r.reason, 
            r.reported_type, 
            r.reported_item_id, 
            r.created_at, 
            r.resolved, 
            r.action_taken,
            u1.username as reporter_username,
            u2.username as reported_username
        FROM 
            reports r
        JOIN 
            users u1 ON r.reporter_id = u1.user_id
        JOIN 
            users u2 ON r.reported_user_id = u2.user_id
        WHERE 
            r.resolved = false
        ORDER BY 
            r.created_at DESC
    "#;

    match sqlx::query(query).fetch_all(pool.get_ref()).await {
        Ok(rows) => {
            let reports = rows
                .iter()
                .map(|row| {
                    json!({
                        "report_id": row.get::<Uuid, _>("report_id"),
                        "reporter_id": row.get::<Uuid, _>("reporter_id"),
                        "reporter_username": row.get::<String, _>("reporter_username"),
                        "reported_user_id": row.get::<Uuid, _>("reported_user_id"),
                        "reported_username": row.get::<String, _>("reported_username"),
                        "reason": row.get::<String, _>("reason"),
                        "reported_type": row.get::<ReportedType, _>("reported_type"),
                        "reported_item_id": row.get::<Uuid, _>("reported_item_id"),
                        "created_at": row.get::<NaiveDateTime, _>("created_at"),
                        "resolved": row.get::<bool, _>("resolved"),
                        "action_taken": row.get::<Option<String>, _>("action_taken"),
                    })
                })
                .collect::<Vec<_>>();

            HttpResponse::Ok().json(reports)
        }
        Err(e) => {
            eprintln!("Database error: {:?}", e);
            HttpResponse::InternalServerError().body("Failed to fetch reports")
        }
    }
}

//Handle Report
//Handle Report Input: HttpRequest(JWT Token), HandleReportRequest
//Handle Report Output: AdminActionResponse
pub async fn handle_report(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    payload: web::Json<HandleReportRequest>,
) -> impl Responder {
    // Check if user is admin
    if let Err(response) = ensure_admin(&req).await {
        return response;
    }

    // Get admin ID from claims
    let admin_id = if let Some(claims) = req.extensions().get::<Claims>() {
        claims.id
    } else {
        return HttpResponse::Unauthorized().body("Authentication required");
    };

    // Start a transaction
    let mut tx = match pool.begin().await {
        Ok(tx) => tx,
        Err(e) => {
            eprintln!("Failed to start transaction: {:?}", e);
            return HttpResponse::InternalServerError().body("Database error");
        }
    };

    // Get report details
    let get_report_query = r#"
        SELECT reporter_id, reported_user_id, reported_type, reported_item_id
        FROM reports
        WHERE report_id = $1
    "#;

    let report = match sqlx::query(get_report_query)
        .bind(payload.report_id)
        .fetch_optional(&mut *tx)
        .await
    {
        Ok(Some(row)) => {
            let reporter_id = row.get::<Uuid, _>("reporter_id");
            let reported_user_id = row.get::<Uuid, _>("reported_user_id");
            let reported_type = row.get::<ReportedType, _>("reported_type");
            let reported_item_id = row.get::<Uuid, _>("reported_item_id");

            (
                reporter_id,
                reported_user_id,
                reported_type,
                reported_item_id,
            )
        }
        Ok(None) => {
            let _ = tx.rollback().await;
            return HttpResponse::NotFound().body("Report not found");
        }
        Err(e) => {
            eprintln!("Failed to get report: {:?}", e);
            let _ = tx.rollback().await;
            return HttpResponse::InternalServerError().body("Database error");
        }
    };

    // Update the report status
    let update_query = r#"
        UPDATE reports
        SET 
            resolved = $1, 
            action_taken = $2,
            resolved_by = $3,
            resolved_at = $4
        WHERE 
            report_id = $5
    "#;

    if let Err(e) = sqlx::query(update_query)
        .bind(payload.resolved)
        .bind(&payload.action_taken)
        .bind(admin_id)
        .bind(Utc::now().naive_utc())
        .bind(payload.report_id)
        .execute(&mut *tx)
        .await
    {
        eprintln!("Failed to update report: {:?}", e);
        let _ = tx.rollback().await;
        return HttpResponse::InternalServerError().body("Failed to update report");
    }

    // Commit the transaction
    if let Err(e) = tx.commit().await {
        eprintln!("Failed to commit transaction: {:?}", e);
        return HttpResponse::InternalServerError().body("Database error");
    }

    // Send notification to the reporter
    let notification_to_reporter = json!({
        "type": "report_handled",
        "data": {
            "report_id": payload.report_id,
            "resolved": payload.resolved,
            "action_taken": payload.action_taken
        }
    });

    // Send WebSocket notification to the reporter
    ws::send_to_user(&report.0, notification_to_reporter).await;

    // Return success response
    HttpResponse::Ok().json(AdminActionResponse {
        success: true,
        message: format!("Report handled successfully"),
    })
}

//Ban User
//Ban User Input: HttpRequest(JWT Token), BanUserRequest
//Ban User Output: AdminActionResponse
pub async fn ban_user(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    payload: web::Json<BanUserRequest>,
) -> impl Responder {
    // Check if user is admin
    if let Err(response) = ensure_admin(&req).await {
        return response;
    }

    // Get admin ID from claims
    let admin_id = if let Some(claims) = req.extensions().get::<Claims>() {
        claims.id
    } else {
        return HttpResponse::Unauthorized().body("Authentication required");
    };

    // Validate input
    if payload.reason.trim().is_empty() {
        return HttpResponse::BadRequest().body("Reason cannot be empty");
    }

    // Calculate ban expiration date if duration is provided
    let banned_until = payload.ban_duration_days.map(|days| {
        Utc::now()
            .checked_add_signed(chrono::Duration::days(days as i64))
            .unwrap_or_else(|| Utc::now())
            .naive_utc()
    });

    // Start a transaction
    let mut tx = match pool.begin().await {
        Ok(tx) => tx,
        Err(e) => {
            eprintln!("Failed to start transaction: {:?}", e);
            return HttpResponse::InternalServerError().body("Database error");
        }
    };

    // Check if user exists and is not already banned
    let check_user_query = r#"
        SELECT username, banned_until
        FROM users
        WHERE user_id = $1
    "#;

    let (username, current_ban) = match sqlx::query(check_user_query)
        .bind(payload.user_id)
        .fetch_optional(&mut *tx)
        .await
    {
        Ok(Some(row)) => {
            let username = row.get::<String, _>("username");
            let banned_until = row.get::<Option<NaiveDateTime>, _>("banned_until");
            (username, banned_until)
        }
        Ok(None) => {
            let _ = tx.rollback().await;
            return HttpResponse::NotFound().body("User not found");
        }
        Err(e) => {
            eprintln!("Failed to check user: {:?}", e);
            let _ = tx.rollback().await;
            return HttpResponse::InternalServerError().body("Database error");
        }
    };

    // Check if user is already banned
    if let Some(ban_time) = current_ban {
        if ban_time > Utc::now().naive_utc() {
            let _ = tx.rollback().await;
            return HttpResponse::BadRequest().body("User is already banned");
        }
    }

    // Update the user's banned_until field
    let update_query = r#"
        UPDATE users
        SET banned_until = $1
        WHERE user_id = $2
    "#;

    if let Err(e) = sqlx::query(update_query)
        .bind(banned_until)
        .bind(payload.user_id)
        .execute(&mut *tx)
        .await
    {
        eprintln!("Failed to ban user: {:?}", e);
        let _ = tx.rollback().await;
        return HttpResponse::InternalServerError().body("Failed to ban user");
    }

    // Log the ban action
    let log_query = r#"
        INSERT INTO admin_actions (action_id, admin_id, user_id, action_type, reason, created_at)
        VALUES ($1, $2, $3, $4, $5, $6)
    "#;

    let ban_type = if banned_until.is_some() {
        "temporary_ban"
    } else {
        "permanent_ban"
    };

    if let Err(e) = sqlx::query(log_query)
        .bind(Uuid::new_v4())
        .bind(admin_id)
        .bind(payload.user_id)
        .bind(ban_type)
        .bind(&payload.reason)
        .bind(Utc::now().naive_utc())
        .execute(&mut *tx)
        .await
    {
        eprintln!("Failed to log ban action: {:?}", e);
        // Continue even if logging fails
    }

    // Commit the transaction
    if let Err(e) = tx.commit().await {
        eprintln!("Failed to commit transaction: {:?}", e);
        return HttpResponse::InternalServerError().body("Database error");
    }

    // Send notification to the banned user
    let notification = json!({
        "type": "user_banned",
        "data": {
            "banned_until": banned_until,
            "reason": payload.reason,
            "ban_type": ban_type
        }
    });

    // Send WebSocket notification to the user
    ws::send_to_user(&payload.user_id, notification).await;

    // Return success response
    let ban_message = if let Some(until) = banned_until {
        format!("User {} banned until {}", username, until)
    } else {
        format!("User {} banned permanently", username)
    };

    HttpResponse::Ok().json(AdminActionResponse {
        success: true,
        message: ban_message,
    })
}

//Unban User
//Unban User Input: HttpRequest(JWT Token), UnbanUserRequest
//Unban User Output: AdminActionResponse
pub async fn unban_user(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    payload: web::Json<UnbanUserRequest>,
) -> impl Responder {
    // Check if user is admin
    if let Err(response) = ensure_admin(&req).await {
        return response;
    }

    // Get admin ID from claims
    let admin_id = if let Some(claims) = req.extensions().get::<Claims>() {
        claims.id
    } else {
        return HttpResponse::Unauthorized().body("Authentication required");
    };

    // Start a transaction
    let mut tx = match pool.begin().await {
        Ok(tx) => tx,
        Err(e) => {
            eprintln!("Failed to start transaction: {:?}", e);
            return HttpResponse::InternalServerError().body("Database error");
        }
    };

    // Check if user exists and is banned
    let check_user_query = r#"
        SELECT username, banned_until
        FROM users
        WHERE user_id = $1
    "#;

    let (username, is_banned) = match sqlx::query(check_user_query)
        .bind(payload.user_id)
        .fetch_optional(&mut *tx)
        .await
    {
        Ok(Some(row)) => {
            let username = row.get::<String, _>("username");
            let banned_until = row.get::<Option<NaiveDateTime>, _>("banned_until");
            let is_banned =
                banned_until.map_or(false, |ban_time| ban_time > Utc::now().naive_utc());
            (username, is_banned)
        }
        Ok(None) => {
            let _ = tx.rollback().await;
            return HttpResponse::NotFound().body("User not found");
        }
        Err(e) => {
            eprintln!("Failed to check user: {:?}", e);
            let _ = tx.rollback().await;
            return HttpResponse::InternalServerError().body("Database error");
        }
    };

    // Check if user is not banned
    if !is_banned {
        let _ = tx.rollback().await;
        return HttpResponse::BadRequest().body("User is not banned");
    }

    // Update the user's banned_until field
    let update_query = r#"
        UPDATE users
        SET banned_until = NULL
        WHERE user_id = $1
    "#;

    if let Err(e) = sqlx::query(update_query)
        .bind(payload.user_id)
        .execute(&mut *tx)
        .await
    {
        eprintln!("Failed to unban user: {:?}", e);
        let _ = tx.rollback().await;
        return HttpResponse::InternalServerError().body("Failed to unban user");
    }

    // Log the unban action
    let log_query = r#"
        INSERT INTO admin_actions (action_id, admin_id, user_id, action_type, reason, created_at)
        VALUES ($1, $2, $3, $4, $5, $6)
    "#;

    if let Err(e) = sqlx::query(log_query)
        .bind(Uuid::new_v4())
        .bind(admin_id)
        .bind(payload.user_id)
        .bind("unban")
        .bind("User unbanned by admin")
        .bind(Utc::now().naive_utc())
        .execute(&mut *tx)
        .await
    {
        eprintln!("Failed to log unban action: {:?}", e);
        // Continue even if logging fails
    }

    // Commit the transaction
    if let Err(e) = tx.commit().await {
        eprintln!("Failed to commit transaction: {:?}", e);
        return HttpResponse::InternalServerError().body("Database error");
    }

    // Send notification to the unbanned user
    let notification = json!({
        "type": "user_unbanned",
        "data": {
            "message": "Your account has been unbanned"
        }
    });

    // Send WebSocket notification to the user
    ws::send_to_user(&payload.user_id, notification).await;

    // Return success response
    HttpResponse::Ok().json(AdminActionResponse {
        success: true,
        message: format!("User {} unbanned successfully", username),
    })
}

//Get Banned Users
//Get Banned Users Input: HttpRequest(JWT Token)
//Get Banned Users Output: Vec<User>
pub async fn get_banned_users(pool: web::Data<PgPool>, req: HttpRequest) -> impl Responder {
    // Check if user is admin
    if let Err(response) = ensure_admin(&req).await {
        return response;
    }

    // Get all banned users
    let query = r#"
        SELECT 
            user_id, 
            username, 
            email, 
            banned_until
        FROM 
            users
        WHERE 
            banned_until IS NOT NULL AND banned_until > $1
        ORDER BY 
            banned_until DESC
    "#;

    match sqlx::query(query)
        .bind(Utc::now().naive_utc())
        .fetch_all(pool.get_ref())
        .await
    {
        Ok(rows) => {
            let banned_users = rows
                .iter()
                .map(|row| {
                    json!({
                        "user_id": row.get::<Uuid, _>("user_id"),
                        "username": row.get::<String, _>("username"),
                        "email": row.get::<String, _>("email"),
                        "banned_until": row.get::<Option<NaiveDateTime>, _>("banned_until"),
                    })
                })
                .collect::<Vec<_>>();

            HttpResponse::Ok().json(banned_users)
        }
        Err(e) => {
            eprintln!("Database error: {:?}", e);
            HttpResponse::InternalServerError().body("Failed to fetch banned users")
        }
    }
}

//Get Admin Stats
//Get Admin Stats Input: HttpRequest(JWT Token)
//Get Admin Stats Output: GetAdminStatsResponse
pub async fn get_admin_stats(pool: web::Data<PgPool>, req: HttpRequest) -> impl Responder {
    // Check if user is admin
    if let Err(response) = ensure_admin(&req).await {
        return response;
    }

    // Start a transaction
    let mut tx = match pool.begin().await {
        Ok(tx) => tx,
        Err(e) => {
            eprintln!("Failed to start transaction: {:?}", e);
            return HttpResponse::InternalServerError().body("Database error");
        }
    };

    // Get total users count
    let total_users_query = "SELECT COUNT(*) FROM users";
    let total_users = match sqlx::query_scalar::<_, i64>(total_users_query)
        .fetch_one(&mut *tx)
        .await
    {
        Ok(count) => count,
        Err(e) => {
            eprintln!("Failed to get total users: {:?}", e);
            let _ = tx.rollback().await;
            return HttpResponse::InternalServerError().body("Database error");
        }
    };

    // Get total sponsors count
    let total_sponsors_query = "SELECT COUNT(*) FROM users WHERE role = 'sponsor'";
    let total_sponsors = match sqlx::query_scalar::<_, i64>(total_sponsors_query)
        .fetch_one(&mut *tx)
        .await
    {
        Ok(count) => count,
        Err(e) => {
            eprintln!("Failed to get total sponsors: {:?}", e);
            let _ = tx.rollback().await;
            return HttpResponse::InternalServerError().body("Database error");
        }
    };

    // Get pending sponsor applications count
    let pending_applications_query =
        "SELECT COUNT(*) FROM sponsor_applications WHERE status = 'pending'";
    let pending_sponsor_applications =
        match sqlx::query_scalar::<_, i64>(pending_applications_query)
            .fetch_one(&mut *tx)
            .await
        {
            Ok(count) => count,
            Err(e) => {
                eprintln!("Failed to get pending applications: {:?}", e);
                let _ = tx.rollback().await;
                return HttpResponse::InternalServerError().body("Database error");
            }
        };

    // Get pending support groups count
    let pending_groups_query = "SELECT COUNT(*) FROM support_groups WHERE status = 'pending'";
    let pending_support_groups = match sqlx::query_scalar::<_, i64>(pending_groups_query)
        .fetch_one(&mut *tx)
        .await
    {
        Ok(count) => count,
        Err(e) => {
            eprintln!("Failed to get pending support groups: {:?}", e);
            let _ = tx.rollback().await;
            return HttpResponse::InternalServerError().body("Database error");
        }
    };

    // Get pending resources count
    let pending_resources_query = "SELECT COUNT(*) FROM resources WHERE approved = false";
    let pending_resources = match sqlx::query_scalar::<_, i64>(pending_resources_query)
        .fetch_one(&mut *tx)
        .await
    {
        Ok(count) => count,
        Err(e) => {
            eprintln!("Failed to get pending resources: {:?}", e);
            let _ = tx.rollback().await;
            return HttpResponse::InternalServerError().body("Database error");
        }
    };

    // Get unresolved reports count
    let unresolved_reports_query = "SELECT COUNT(*) FROM reports WHERE resolved = false";
    let unresolved_reports = match sqlx::query_scalar::<_, i64>(unresolved_reports_query)
        .fetch_one(&mut *tx)
        .await
    {
        Ok(count) => count,
        Err(e) => {
            eprintln!("Failed to get unresolved reports: {:?}", e);
            let _ = tx.rollback().await;
            return HttpResponse::InternalServerError().body("Database error");
        }
    };

    // Commit the transaction
    if let Err(e) = tx.commit().await {
        eprintln!("Failed to commit transaction: {:?}", e);
        return HttpResponse::InternalServerError().body("Database error");
    }

    // Return the statistics
    HttpResponse::Ok().json(GetAdminStatsResponse {
        total_users,
        total_sponsors,
        pending_sponsor_applications,
        pending_support_groups,
        pending_resources,
        unresolved_reports,
    })
}

//Config Admin Routes
// GET /admin/sponsor-applications
// POST /admin/sponsor-applications/review
// GET /admin/support-groups
// POST /admin/support-groups/review
// GET /admin/resources
// POST /admin/resources/review
// GET /admin/reports
// POST /admin/reports/handle
// POST /admin/users/ban
// POST /admin/users/unban
// GET /admin/users/banned
// GET /admin/stats
pub fn config_admin_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/admin")
            // Sponsor application routes
            .route(
                "/sponsor-applications/pending",
                web::get().to(get_pending_sponsor_applications),
            )
            .route(
                "/sponsor-applications/review",
                web::post().to(review_sponsor_application),
            )
            // Support group routes
            .route(
                "/support-groups/pending",
                web::get().to(get_pending_support_groups),
            )
            .route(
                "/support-groups/review",
                web::post().to(review_support_group),
            )
            // Resource routes
            .route("/resources/pending", web::get().to(get_pending_resources))
            .route("/resources/review", web::post().to(review_resource))
            // Report routes
            .route("/reports/unresolved", web::get().to(get_unresolved_reports))
            .route("/reports/handle", web::post().to(handle_report))
            // User management routes
            .route("/users/ban", web::post().to(ban_user))
            .route("/users/unban", web::post().to(unban_user))
            .route("/users/banned", web::get().to(get_banned_users))
            // Admin dashboard routes
            .route("/stats", web::get().to(get_admin_stats)),
    );
}
