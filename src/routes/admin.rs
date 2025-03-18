use crate::handlers::auth::Claims;
use crate::models::all_models::{
    ApplicationStatus, ReportStatus, ReportedType, SupportGroupStatus, UserRole,
};
use actix_web::{web, HttpMessage, HttpRequest, HttpResponse, Responder};
use chrono::{NaiveDate, NaiveDateTime, Utc};
use log::error;
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
    pub status: SupportGroupStatus,
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

//Get Banned Users Query Params
#[derive(Debug, Deserialize)]
pub struct GetAllUsersParams {
    pub username: Option<String>,
    pub role: Option<UserRole>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

//Ensure Admin Helper Function
fn ensure_admin(req: &HttpRequest) -> Result<(), HttpResponse> {
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

fn get_user_id_from_request(req: &HttpRequest) -> Option<Uuid> {
    req.extensions().get::<Claims>().map(|claims| claims.id)
}

//Get Pending Sponsor Applications
//Get Pending Sponsor Applications Input: HttpRequest(JWT Token)
//Get Pending Sponsor Applications Output: Vec<SponsorApplication>
pub async fn get_pending_sponsor_applications(
    pool: web::Data<PgPool>,
    req: HttpRequest,
) -> impl Responder {
    // Check if user is admin
    if let Err(response) = ensure_admin(&req) {
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
            sa.status = $1
        ORDER BY 
            sa.created_at DESC
    "#;

    match sqlx::query(query)
        .bind(ApplicationStatus::Pending)
        .fetch_all(pool.get_ref())
        .await
    {
        Ok(rows) => {
            let applications = rows
                .iter()
                .map(|row| {
                    // Parse the application_info from TEXT to JSON
                    let application_info_str: String = row.get("application_info");
                    let application_info = match serde_json::from_str(&application_info_str) {
                        Ok(json) => json,
                        Err(e) => {
                            error!("Failed to parse application_info as JSON: {}", e);
                            error!("Raw application_info: {}", application_info_str);
                            serde_json::json!({})
                        }
                    };

                    json!({
                        "application_id": row.get::<Uuid, _>("application_id"),
                        "user_id": row.get::<Uuid, _>("user_id"),
                        "username": row.get::<String, _>("username"),
                        "email": row.get::<String, _>("email"),
                        "status": row.get::<ApplicationStatus, _>("status"),
                        "application_info": application_info,
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
    if let Err(response) = ensure_admin(&req) {
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
            SET role = $1
            WHERE user_id = $2
        "#;

        if let Err(e) = sqlx::query(update_user_query)
            .bind(UserRole::Sponsor)
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
    if let Err(response) = ensure_admin(&req) {
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
        LEFT JOIN 
            users u ON sg.admin_id = u.user_id
        WHERE 
            sg.status = $1
        ORDER BY 
            sg.created_at DESC
    "#;

    match sqlx::query(query)
        .bind(SupportGroupStatus::Pending)
        .fetch_all(pool.get_ref())
        .await
    {
        Ok(rows) => {
            let support_groups = rows
                .iter()
                .map(|row| {
                    json!({
                        "support_group_id": row.get::<Uuid, _>("support_group_id"),
                        "title": row.get::<String, _>("title"),
                        "description": row.get::<String, _>("description"),
                        "admin_id": row.get::<Option<Uuid>, _>("admin_id"),
                        "admin_username": row.try_get::<String, _>("username").ok(),
                        "admin_email": row.try_get::<String, _>("email").ok(),
                        "group_chat_id": row.get::<Option<Uuid>, _>("group_chat_id"),
                        "status": row.get::<SupportGroupStatus, _>("status"),
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
    if let Err(response) = ensure_admin(&req) {
        return response;
    }

    // Get admin ID from claims
    let admin_id = if let Some(claims) = req.extensions().get::<Claims>() {
        claims.id
    } else {
        return HttpResponse::Unauthorized().body("Authentication required");
    };

    // Validate input
    if payload.status != SupportGroupStatus::Approved
        && payload.status != SupportGroupStatus::Rejected
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

    // Check if the support group exists
    let check_group_query = r#"
        SELECT EXISTS(SELECT 1 FROM support_groups WHERE support_group_id = $1)
    "#;

    let group_exists = match sqlx::query_scalar::<_, bool>(check_group_query)
        .bind(payload.support_group_id)
        .fetch_one(&mut *tx)
        .await
    {
        Ok(exists) => exists,
        Err(e) => {
            eprintln!("Failed to check if support group exists: {:?}", e);
            let _ = tx.rollback().await;
            return HttpResponse::InternalServerError().body("Database error");
        }
    };

    if !group_exists {
        let _ = tx.rollback().await;
        return HttpResponse::NotFound().body("Support group not found");
    }

    // Variable to store group_chat_id if needed
    let mut group_chat_id = None;

    // If approved, create a group chat for the support group if it doesn't exist
    if payload.status == SupportGroupStatus::Approved {
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
                    INSERT INTO group_chats (group_chat_id, creator_id, created_at, flagged)
                    VALUES ($1, $2, $3, false)
                "#;

                // Get support group title for logging purposes
                let get_title_query = r#"
                    SELECT title
                    FROM support_groups
                    WHERE support_group_id = $1
                "#;

                match sqlx::query_scalar::<_, String>(get_title_query)
                    .bind(payload.support_group_id)
                    .fetch_one(&mut *tx)
                    .await
                {
                    Ok(_) => {} // We don't need to use the title, just checking it exists
                    Err(e) => {
                        eprintln!("Failed to get support group title: {:?}", e);
                        let _ = tx.rollback().await;
                        return HttpResponse::InternalServerError().body("Database error");
                    }
                };

                if let Err(e) = sqlx::query(create_chat_query)
                    .bind(new_chat_id)
                    .bind(admin_id)
                    .bind(Utc::now().naive_utc())
                    .execute(&mut *tx)
                    .await
                {
                    eprintln!("Failed to create group chat: {:?}", e);
                    let _ = tx.rollback().await;
                    return HttpResponse::InternalServerError().body("Failed to create group chat");
                }

                // Add the admin as a member of the group chat
                let add_member_query = r#"
                    INSERT INTO group_chat_members (group_chat_id, user_id)
                    VALUES ($1, $2)
                "#;

                if let Err(e) = sqlx::query(add_member_query)
                    .bind(new_chat_id)
                    .bind(admin_id)
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
    let update_query = if payload.status == SupportGroupStatus::Approved {
        // For approved groups, set the admin_id to the current admin
        r#"
            UPDATE support_groups
            SET 
                status = $1,
                group_chat_id = $2,
                admin_id = $3
            WHERE 
                support_group_id = $4
        "#
    } else {
        // For rejected groups, just update the status
        r#"
            UPDATE support_groups
            SET 
                status = $1,
                group_chat_id = $2
            WHERE 
                support_group_id = $3
        "#
    };

    // Execute the appropriate query based on approval status
    if payload.status == SupportGroupStatus::Approved {
        if let Err(e) = sqlx::query(update_query)
            .bind(&payload.status)
            .bind(group_chat_id)
            .bind(admin_id)
            .bind(payload.support_group_id)
            .execute(&mut *tx)
            .await
        {
            eprintln!("Failed to update support group: {:?}", e);
            let _ = tx.rollback().await;
            return HttpResponse::InternalServerError().body("Failed to update support group");
        }
    } else {
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
    }

    // Commit the transaction
    if let Err(e) = tx.commit().await {
        eprintln!("Failed to commit transaction: {:?}", e);
        return HttpResponse::InternalServerError().body("Database error");
    }

    // Return success response
    HttpResponse::Ok().json(AdminActionResponse {
        success: true,
        message: format!("Support group {:?} successfully", payload.status),
    })
}

//Get Pending Resources
//Get Pending Resources Input: HttpRequest(JWT Token)
//Get Pending Resources Output: Vec<Resource>
pub async fn get_pending_resources(pool: web::Data<PgPool>, req: HttpRequest) -> impl Responder {
    // Check if user is admin
    if let Err(response) = ensure_admin(&req) {
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
    if let Err(response) = ensure_admin(&req) {
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

    let _contributor_id = match sqlx::query_scalar::<_, Uuid>(get_contributor_query)
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

    let _resource_title = match sqlx::query_scalar::<_, String>(update_query)
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
    if let Err(response) = ensure_admin(&req) {
        return response;
    }

    // Get all unresolved reports
    let query = r#"
        SELECT 
            r.report_id, 
            r.reporter_id,
            r.reported_user_id,
            r.reason as description,
            r.reported_type as report_type,
            r.reported_item_id,
            r.status,
            r.reviewed_by,
            r.created_at,
            reporter.username as reporter_username,
            CASE 
                WHEN r.reported_type = $3 THEN reported.username 
                ELSE NULL 
            END as reported_username,
            CASE 
                WHEN r.status = $1 THEN 'Medium'
                WHEN r.status = $2 THEN 'Low'
                ELSE 'High'
            END as severity
        FROM 
            reports r
        JOIN 
            users reporter ON r.reporter_id = reporter.user_id
        LEFT JOIN 
            users reported ON r.reported_user_id = reported.user_id
        WHERE 
            r.status = $1
        ORDER BY 
            r.created_at DESC
    "#;

    match sqlx::query(query)
        .bind(ReportStatus::Pending)
        .bind(ReportStatus::Resolved)
        .bind(ReportedType::User)
        .fetch_all(pool.get_ref())
        .await
    {
        Ok(rows) => {
            let reports = rows
                .iter()
                .map(|row| {
                    json!({
                        "id": row.get::<Uuid, _>("report_id").to_string(),
                        "reporter_id": row.get::<Uuid, _>("reporter_id").to_string(),
                        "reported_item_id": row.get::<Uuid, _>("reported_item_id").to_string(),
                        "description": row.get::<String, _>("description"),
                        "report_type": row.get::<String, _>("report_type"),
                        "status": row.get::<String, _>("status"),
                        "created_at": row.get::<NaiveDateTime, _>("created_at"),
                        "reporter_username": row.get::<Option<String>, _>("reporter_username"),
                        "reported_username": row.get::<Option<String>, _>("reported_username"),
                        "severity": row.get::<String, _>("severity")
                    })
                })
                .collect::<Vec<serde_json::Value>>();

            HttpResponse::Ok().json(json!({
                "success": true,
                "data": reports
            }))
        }
        Err(e) => {
            error!("Failed to get unresolved reports: {}", e);
            HttpResponse::InternalServerError().json(json!({
                "success": false,
                "message": "Failed to get unresolved reports"
            }))
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
    if let Err(response) = ensure_admin(&req) {
        return response;
    }

    let user_id = match get_user_id_from_request(&req) {
        Some(id) => id,
        None => {
            return HttpResponse::Unauthorized().json(json!({
                "success": false,
                "message": "Unauthorized"
            }));
        }
    };

    // Update report status
    let query = r#"
        UPDATE reports
        SET 
            status = $1,
            reviewed_by = $2,
            resolved_at = CASE WHEN $3 THEN NOW() ELSE NULL END
        WHERE 
            report_id = $4
        RETURNING report_id
    "#;

    let status = if payload.resolved {
        ReportStatus::Resolved
    } else {
        ReportStatus::Pending
    };

    match sqlx::query(query)
        .bind(status)
        .bind(user_id)
        .bind(payload.resolved)
        .bind(payload.report_id)
        .fetch_optional(pool.get_ref())
        .await
    {
        Ok(Some(_)) => {
            // Record the action taken
            let action_query = r#"
                INSERT INTO admin_actions (admin_id, action_type, target_id, details)
                VALUES ($1, 'handle_report', $2, $3)
            "#;

            let _ = sqlx::query(action_query)
                .bind(user_id)
                .bind(payload.report_id)
                .bind(payload.action_taken.clone())
                .execute(pool.get_ref())
                .await;

            HttpResponse::Ok().json(AdminActionResponse {
                success: true,
                message: "Report handled successfully".to_string(),
            })
        }
        Ok(None) => HttpResponse::NotFound().json(json!({
            "success": false,
            "message": "Report not found"
        })),
        Err(e) => {
            error!("Failed to handle report: {}", e);
            HttpResponse::InternalServerError().json(json!({
                "success": false,
                "message": "Failed to handle report"
            }))
        }
    }
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
    if let Err(response) = ensure_admin(&req) {
        return response;
    }

   

    // Validate input
    if payload.reason.trim().is_empty() {
        return HttpResponse::BadRequest().body("Reason cannot be empty");
    }

    // Calculate ban expiration date
    // For permanent bans (when ban_duration_days is None or negative), use year 9999
    let banned_until = match payload.ban_duration_days {
        Some(days) if days > 0 => {
            // Temporary ban with specific duration
            Utc::now()
                .checked_add_signed(chrono::Duration::days(days as i64))
                .unwrap_or_else(|| Utc::now())
                .naive_utc()
        }
        _ => {
            // Permanent ban - use year 9999
            NaiveDateTime::new(
                chrono::NaiveDate::from_ymd_opt(9999, 12, 31).unwrap(),
                chrono::NaiveTime::from_hms_opt(23, 59, 59).unwrap(),
            )
        }
    };

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

    // Update the user's banned_until field with the calculated date
    let update_query = r#"
        UPDATE users
        SET banned_until = $1
        WHERE user_id = $2
    "#;

    if let Err(e) = sqlx::query(update_query)
        .bind(banned_until) // Always bind a date, never NULL
        .bind(payload.user_id)
        .execute(&mut *tx)
        .await
    {
        eprintln!("Failed to ban user: {:?}", e);
        let _ = tx.rollback().await;
        return HttpResponse::InternalServerError().body("Failed to ban user");
    }

    // Commit the transaction
    if let Err(e) = tx.commit().await {
        eprintln!("Failed to commit transaction: {:?}", e);
        return HttpResponse::InternalServerError().body("Database error");
    }

    // Return success response
    let ban_message =
        if payload.ban_duration_days.is_some() && payload.ban_duration_days.unwrap() > 0 {
            format!("User {} banned until {}", username, banned_until)
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
    if let Err(response) = ensure_admin(&req) {
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

    // Commit the transaction
    if let Err(e) = tx.commit().await {
        eprintln!("Failed to commit transaction: {:?}", e);
        return HttpResponse::InternalServerError().body("Database error");
    }

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
    if let Err(response) = ensure_admin(&req) {
        return response;
    }

    // Get all banned users
    let query = r#"
        SELECT 
            user_id, 
            username, 
            email, 
            banned_until,
            CASE 
                WHEN EXTRACT(YEAR FROM banned_until) = 9999 THEN true
                ELSE false
            END as is_permanent_ban
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
                        "is_permanent_ban": row.get::<bool, _>("is_permanent_ban")
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

//Get All Users
//Get All Users Input: HttpRequest(JWT Token), GetAllUsersParams
//Get All Users Output: Vec<User>
pub async fn get_all_users(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    query: web::Query<GetAllUsersParams>,
) -> impl Responder {
    // Check if user is admin
    if let Err(response) = ensure_admin(&req) {
        return response;
    }

    let limit = query.limit.unwrap_or(100); // Default to 100 users per page
    let offset = query.offset.unwrap_or(0);

    // Get username from the query params for search
    let username_pattern = query.username.as_ref().map(|u| format!("%{}%", u));

    // Build the SQL query based on the parameters provided
    let users_result = if let Some(role) = &query.role {
        // Search with role filter
        if let Some(username) = &username_pattern {
            // Search by both username and role
            sqlx::query(
                r#"
                SELECT 
                    user_id, username, email, role, banned_until, avatar_url, created_at, dob, 
                    email_verified, privacy,
                    CASE WHEN banned_until IS NOT NULL AND banned_until > NOW() THEN true ELSE false END as is_banned
                FROM users
                WHERE username ILIKE $1 AND role = $2
                ORDER BY created_at DESC
                LIMIT $3 OFFSET $4
                "#
            )
            .bind(username)
            .bind(role)
            .bind(limit)
            .bind(offset)
            .fetch_all(pool.get_ref())
            .await
        } else {
            // Search by role only
            sqlx::query(
                r#"
                SELECT 
                    user_id, username, email, role, banned_until, avatar_url, created_at, dob, 
                    email_verified, privacy,
                    CASE WHEN banned_until IS NOT NULL AND banned_until > NOW() THEN true ELSE false END as is_banned
                FROM users
                WHERE role = $1
                ORDER BY created_at DESC
                LIMIT $2 OFFSET $3
                "#
            )
            .bind(role)
            .bind(limit)
            .bind(offset)
            .fetch_all(pool.get_ref())
            .await
        }
    } else if let Some(username) = &username_pattern {
        // Search by username only
        sqlx::query(
            r#"
            SELECT 
                user_id, username, email, role, banned_until, avatar_url, created_at, dob, 
                email_verified, privacy,
                CASE WHEN banned_until IS NOT NULL AND banned_until > NOW() THEN true ELSE false END as is_banned
            FROM users
            WHERE username ILIKE $1
            ORDER BY created_at DESC
            LIMIT $2 OFFSET $3
            "#
        )
        .bind(username)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool.get_ref())
        .await
    } else {
        // No filters, get all users
        sqlx::query(
            r#"
            SELECT 
                user_id, username, email, role, banned_until, avatar_url, created_at, dob, 
                email_verified, privacy,
                CASE WHEN banned_until IS NOT NULL AND banned_until > NOW() THEN true ELSE false END as is_banned
            FROM users
            ORDER BY created_at DESC
            LIMIT $1 OFFSET $2
            "#
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(pool.get_ref())
        .await
    };

    // Get the total count with the same filters
    let count_result = if let Some(role) = &query.role {
        if let Some(username) = &username_pattern {
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM users WHERE username ILIKE $1 AND role = $2",
            )
            .bind(username)
            .bind(role)
            .fetch_one(pool.get_ref())
            .await
        } else {
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM users WHERE role = $1")
                .bind(role)
                .fetch_one(pool.get_ref())
                .await
        }
    } else if let Some(username) = &username_pattern {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM users WHERE username ILIKE $1")
            .bind(username)
            .fetch_one(pool.get_ref())
            .await
    } else {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM users")
            .fetch_one(pool.get_ref())
            .await
    };

    match (users_result, count_result) {
        (Ok(rows), Ok(total_count)) => {
            let users = rows
                .iter()
                .map(|row| {
                    json!({
                        "user_id": row.get::<Uuid, _>("user_id"),
                        "username": row.get::<String, _>("username"),
                        "email": row.get::<String, _>("email"),
                        "role": row.get::<UserRole, _>("role"),
                        "avatar_url": row.get::<String, _>("avatar_url"),
                        "created_at": row.get::<NaiveDateTime, _>("created_at"),
                        "dob": row.get::<NaiveDate, _>("dob"),
                        "email_verified": row.get::<bool, _>("email_verified"),
                        "privacy": row.get::<bool, _>("privacy"),
                        "is_banned": row.get::<bool, _>("is_banned"),
                        "banned_until": row.get::<Option<NaiveDateTime>, _>("banned_until")
                    })
                })
                .collect::<Vec<_>>();

            HttpResponse::Ok().json(json!({
                "users": users,
                "total": total_count,
                "limit": limit,
                "offset": offset
            }))
        }
        (Err(e), _) | (_, Err(e)) => {
            error!("Database error: {:?}", e);
            HttpResponse::InternalServerError().json(json!({
                "success": false,
                "message": "Failed to fetch users"
            }))
        }
    }
}

//Get Admin Stats
//Get Admin Stats Input: HttpRequest(JWT Token)
//Get Admin Stats Output: GetAdminStatsResponse
pub async fn get_admin_stats(pool: web::Data<PgPool>, req: HttpRequest) -> impl Responder {
    // Check if user is admin
    if let Err(response) = ensure_admin(&req) {
        return response;
    }

    // Get user counts
    let user_counts_query = r#"
        SELECT
            COUNT(*) as total_users,
            COUNT(CASE WHEN role = $1 THEN 1 END) as member_users,
            COUNT(CASE WHEN role = $2 THEN 1 END) as sponsor_users,
            COUNT(CASE WHEN role = $3 THEN 1 END) as admin_users,
            COUNT(CASE WHEN banned_until IS NOT NULL AND banned_until > NOW() THEN 1 END) as banned_users
        FROM users
    "#;

    // Get resource counts
    let resource_counts_query = r#"
        SELECT
            COUNT(*) as total,
            COUNT(CASE WHEN content ILIKE '%article%' THEN 1 END) as articles,
            COUNT(CASE WHEN content ILIKE '%video%' THEN 1 END) as videos,
            COUNT(CASE WHEN content ILIKE '%podcast%' THEN 1 END) as podcasts,
            COUNT(CASE WHEN content ILIKE '%book%' THEN 1 END) as books,
            COUNT(CASE WHEN 
                content NOT ILIKE '%article%' AND 
                content NOT ILIKE '%video%' AND 
                content NOT ILIKE '%podcast%' AND 
                content NOT ILIKE '%book%' 
                THEN 1 END) as other
        FROM resources
    "#;

    // Get support group counts
    let support_group_counts_query = r#"
        SELECT
            COUNT(*) as total
        FROM support_groups
    "#;

    // Get report counts
    let report_counts_query = r#"
        SELECT
            COUNT(*) as total,
            COUNT(CASE WHEN status = $1 THEN 1 END) as resolved,
            COUNT(CASE WHEN status = $2 THEN 1 END) as pending
        FROM reports
    "#;

    // Get monthly user registrations (last 6 months)
    let user_registrations_query = r#"
        SELECT
            TO_CHAR(DATE_TRUNC('month', created_at), 'Mon YYYY') as month,
            COUNT(*) as count
        FROM users
        WHERE created_at > NOW() - INTERVAL '6 months'
        GROUP BY DATE_TRUNC('month', created_at)
        ORDER BY DATE_TRUNC('month', created_at)
    "#;

    // Get pending sponsor applications count
    let pending_sponsor_applications_query = r#"
        SELECT COUNT(*) as count
        FROM sponsor_applications
        WHERE status = $1
    "#;

    // Get pending support groups count
    let pending_support_groups_query = r#"
        SELECT COUNT(*) as count
        FROM support_groups
        WHERE status = $1
    "#;

    // Get pending resources count
    let pending_resources_query = r#"
        SELECT COUNT(*) as count
        FROM resources
        WHERE approved = false
    "#;

    // Execute queries individually instead of using try_join6
    let user_counts_result = sqlx::query(user_counts_query)
        .bind(UserRole::Member)
        .bind(UserRole::Sponsor)
        .bind(UserRole::Admin)
        .fetch_one(pool.get_ref())
        .await;
    let resource_counts_result = sqlx::query(resource_counts_query)
        .fetch_one(pool.get_ref())
        .await;
    let support_group_counts_result = sqlx::query(support_group_counts_query)
        .fetch_one(pool.get_ref())
        .await;
    let report_counts_result = sqlx::query(report_counts_query)
        .bind(ReportStatus::Resolved)
        .bind(ReportStatus::Pending)
        .fetch_one(pool.get_ref())
        .await;
    let user_registrations_result = sqlx::query(user_registrations_query)
        .fetch_all(pool.get_ref())
        .await;
    let pending_sponsor_applications_result = sqlx::query(pending_sponsor_applications_query)
        .bind(ApplicationStatus::Pending)
        .fetch_one(pool.get_ref())
        .await;
    let pending_support_groups_result = sqlx::query(pending_support_groups_query)
        .bind(SupportGroupStatus::Pending)
        .fetch_one(pool.get_ref())
        .await;
    let pending_resources_result = sqlx::query(pending_resources_query)
        .fetch_one(pool.get_ref())
        .await;

    // Check if any query failed
    if let Err(e) = &user_counts_result {
        error!("Failed to get user counts: {}", e);
        return HttpResponse::InternalServerError().json(json!({
            "success": false,
            "message": "Failed to get admin stats"
        }));
    }
    if let Err(e) = &resource_counts_result {
        error!("Failed to get resource counts: {}", e);
        return HttpResponse::InternalServerError().json(json!({
            "success": false,
            "message": "Failed to get admin stats"
        }));
    }
    if let Err(e) = &support_group_counts_result {
        error!("Failed to get support group counts: {}", e);
        return HttpResponse::InternalServerError().json(json!({
            "success": false,
            "message": "Failed to get admin stats"
        }));
    }
    if let Err(e) = &report_counts_result {
        error!("Failed to get report counts: {}", e);
        return HttpResponse::InternalServerError().json(json!({
            "success": false,
            "message": "Failed to get admin stats"
        }));
    }
    if let Err(e) = &user_registrations_result {
        error!("Failed to get user registrations: {}", e);
        return HttpResponse::InternalServerError().json(json!({
            "success": false,
            "message": "Failed to get admin stats"
        }));
    }

    if let Err(e) = &pending_sponsor_applications_result {
        error!("Failed to get pending sponsor applications: {}", e);
        return HttpResponse::InternalServerError().json(json!({
            "success": false,
            "message": "Failed to get admin stats"
        }));
    }
    if let Err(e) = &pending_support_groups_result {
        error!("Failed to get pending support groups: {}", e);
        return HttpResponse::InternalServerError().json(json!({
            "success": false,
            "message": "Failed to get admin stats"
        }));
    }
    if let Err(e) = &pending_resources_result {
        error!("Failed to get pending resources: {}", e);
        return HttpResponse::InternalServerError().json(json!({
            "success": false,
            "message": "Failed to get admin stats"
        }));
    }

    // Unwrap results
    let user_counts = user_counts_result.unwrap();
    let resource_counts = resource_counts_result.unwrap();
    let support_group_counts = support_group_counts_result.unwrap();
    let report_counts = report_counts_result.unwrap();
    let user_registrations = user_registrations_result.unwrap();
    let pending_sponsor_applications = pending_sponsor_applications_result.unwrap();
    let pending_support_groups = pending_support_groups_result.unwrap();
    let pending_resources = pending_resources_result.unwrap();

    // Build user counts object
    let user_counts_obj = json!({
        "totalUsers": user_counts.get::<i64, _>("total_users"),
        "memberUsers": user_counts.get::<i64, _>("member_users"),
        "sponsorUsers": user_counts.get::<i64, _>("sponsor_users"),
        "adminUsers": user_counts.get::<i64, _>("admin_users"),
        "bannedUsers": user_counts.get::<i64, _>("banned_users")
    });

    // Build resource counts object
    let resource_counts_obj = json!({
        "total": resource_counts.get::<i64, _>("total"),
        "articles": resource_counts.get::<i64, _>("articles"),
        "videos": resource_counts.get::<i64, _>("videos"),
        "podcasts": resource_counts.get::<i64, _>("podcasts"),
        "books": resource_counts.get::<i64, _>("books"),
        "other": resource_counts.get::<i64, _>("other")
    });

    // Build support group counts object
    let support_group_counts_obj = json!({
        "total": support_group_counts.get::<i64, _>("total")
    });

    // Build report counts object
    let report_counts_obj = json!({
        "total": report_counts.get::<i64, _>("total"),
        "resolved": report_counts.get::<i64, _>("resolved"),
        "pending": report_counts.get::<i64, _>("pending")
    });

    // Build user registrations array
    let user_registrations_arr = user_registrations
        .iter()
        .map(|row| {
            json!({
                "month": row.get::<String, _>("month"),
                "count": row.get::<i64, _>("count")
            })
        })
        .collect::<Vec<serde_json::Value>>();

    // Build the complete response
    let response = json!({
        "userCounts": user_counts_obj,
        "resourceCounts": resource_counts_obj,
        "supportGroupCounts": support_group_counts_obj,
        "reportCounts": report_counts_obj,
        "userRegistrationsByMonth": user_registrations_arr,

        // Include the original flat structure with actual values
        "total_users": user_counts.get::<i64, _>("total_users"),
        "total_sponsors": user_counts.get::<i64, _>("sponsor_users"),
        "pending_sponsor_applications": pending_sponsor_applications.get::<i64, _>("count"),
        "pending_support_groups": pending_support_groups.get::<i64, _>("count"),
        "pending_resources": pending_resources.get::<i64, _>("count"),
        "unresolved_reports": report_counts.get::<i64, _>("pending")
    });

    HttpResponse::Ok().json(json!({
        "success": true,
        "data": response
    }))
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
// GET /admin/users
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
            .route("/users", web::get().to(get_all_users))
            // Admin dashboard routes
            .route("/stats", web::get().to(get_admin_stats)),
    );
}
