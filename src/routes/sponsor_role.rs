use crate::handlers::auth::Claims;
use crate::handlers::ws::send_to_role;
use crate::models::all_models::{ApplicationStatus, UserRole};
use actix_web::{web, HttpMessage, HttpRequest, HttpResponse, Responder};
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

//Sponsor Application Request
#[derive(Debug, Deserialize, Serialize)]
pub struct SponsorApplicationRequest {
    pub application_info: String,
}

//Sponsor Application
#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct SponsorApplication {
    pub application_id: Uuid,
    pub user_id: Uuid,
    pub status: ApplicationStatus,
    pub application_info: String,
    pub reviewed_by: Option<Uuid>,
    pub admin_comments: Option<String>,
    pub created_at: NaiveDateTime,
}

//Submit Sponsor Application
//Submit Sponsor Application Input: HttpRequest(JWT Token), SponsorApplicationRequest
//Submit Sponsor Application Output: SponsorApplication
pub async fn submit_sponsor_application(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    payload: web::Json<SponsorApplicationRequest>,
) -> impl Responder {
    if let Some(claims) = req.extensions().get::<Claims>() {
        // Check if the user has already submitted an application
        let check_query = "SELECT status FROM sponsor_applications WHERE user_id = $1";

        let existing_status = sqlx::query_scalar::<_, Option<ApplicationStatus>>(check_query)
            .bind(&claims.id)
            .fetch_one(pool.get_ref())
            .await;

        match existing_status {
            Ok(Some(status)) => {
                // If application exists and is approved, user cannot reapply
                if status == ApplicationStatus::Approved {
                    return HttpResponse::Forbidden()
                        .body("You already have an approved sponsor application.");
                }
                // If application exists but is pending or rejected, user should update instead
                return HttpResponse::Conflict().body(
                    "You already have an application. Please use the update endpoint instead.",
                );
            }
            Ok(None) | Err(_) => {
                // No existing application, proceed with creating a new one
                let insert_query = "
                    INSERT INTO sponsor_applications (user_id, status, application_info, created_at)
                    VALUES ($1, $2, $3, NOW())
                    RETURNING application_id, user_id, status, application_info, reviewed_by, admin_comments, created_at";

                let application_result = sqlx::query_as::<_, SponsorApplication>(insert_query)
                    .bind(&claims.id)
                    .bind(ApplicationStatus::Pending)
                    .bind(&payload.application_info)
                    .fetch_one(pool.get_ref())
                    .await;

                match application_result {
                    Ok(application) => {
                        // Create notification payload
                        let notification = json!({
                            "type": "new_sponsor_application",
                            "data": {
                                "application_id": application.application_id,
                                "user_id": application.user_id,
                                "status": application.status,
                                "application_info": application.application_info,
                                "created_at": application.created_at
                            }
                        });

                        // Send notification to admin users via websocket
                        let admin_role = UserRole::Admin;
                        let _ = send_to_role(&admin_role, notification).await;

                        HttpResponse::Ok().json(application)
                    }
                    Err(_) => {
                        HttpResponse::InternalServerError().body("Failed to submit application")
                    }
                }
            }
        }
    } else {
        HttpResponse::Unauthorized().body("Authentication required")
    }
}

//Check Sponsor Application Status
//Check Sponsor Application Status Input: HttpRequest(JWT Token)
//Check Sponsor Application Status Output: SponsorApplication
pub async fn check_sponsor_application_status(
    pool: web::Data<PgPool>,
    req: HttpRequest,
) -> impl Responder {
    if let Some(claims) = req.extensions().get::<Claims>() {
        let query = "SELECT * FROM sponsor_applications WHERE user_id = $1";

        let result = sqlx::query_as::<_, SponsorApplication>(query)
            .bind(&claims.id)
            .fetch_one(pool.get_ref())
            .await;

        match result {
            Ok(application) => HttpResponse::Ok().json(application),
            Err(_) => HttpResponse::NotFound().body("No sponsor application found."),
        }
    } else {
        HttpResponse::Unauthorized().body("Authentication required")
    }
}

//Update Sponsor Application Request
#[derive(Debug, Deserialize, Serialize)]
pub struct UpdateSponsorApplicationRequest {
    pub application_info: String,
}

//Update Sponsor Application
//Update Sponsor Application Input: HttpRequest(JWT Token), UpdateSponsorApplicationRequest
//Update Sponsor Application Output: SponsorApplication
pub async fn update_sponsor_application(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    payload: web::Json<UpdateSponsorApplicationRequest>,
) -> impl Responder {
    if let Some(claims) = req.extensions().get::<Claims>() {
        // Check if application exists and get its status
        let check_query = "SELECT status FROM sponsor_applications WHERE user_id = $1";

        let result: Result<ApplicationStatus, sqlx::Error> = sqlx::query_scalar(check_query)
            .bind(&claims.id)
            .fetch_one(pool.get_ref())
            .await;

        match result {
            Ok(status) => {
                // If application is approved, user cannot update it
                if status == ApplicationStatus::Approved {
                    return HttpResponse::Forbidden()
                        .body("You cannot update an approved application.");
                }

                // Update application - if rejected, set back to pending
                let update_query = "
                    UPDATE sponsor_applications 
                    SET application_info = $1, status = CASE WHEN status = 'rejected' THEN 'pending' ELSE status END 
                    WHERE user_id = $2
                    RETURNING application_id, user_id, status, application_info, reviewed_by, admin_comments, created_at";

                let updated_result = sqlx::query_as::<_, SponsorApplication>(update_query)
                    .bind(&payload.application_info)
                    .bind(&claims.id)
                    .fetch_one(pool.get_ref())
                    .await;

                match updated_result {
                    Ok(application) => {
                        // Create notification payload
                        let notification = json!({
                            "type": "updated_sponsor_application",
                            "data": {
                                "application_id": application.application_id,
                                "user_id": application.user_id,
                                "status": application.status,
                                "application_info": application.application_info,
                                "created_at": application.created_at
                            }
                        });

                        // Send notification to admin users via websocket
                        let admin_role = UserRole::Admin;
                        let _ = send_to_role(&admin_role, notification).await;

                        HttpResponse::Ok().json(application)
                    }
                    Err(_) => {
                        HttpResponse::InternalServerError().body("Failed to update application.")
                    }
                }
            }
            Err(_) => HttpResponse::NotFound()
                .body("No sponsor application found. Please submit an application first."),
        }
    } else {
        HttpResponse::Unauthorized().body("Authentication required")
    }
}

//Delete Sponsor Application
//Delete Sponsor Application Input: HttpRequest(JWT Token)
//Delete Sponsor Application Output: Success message
pub async fn delete_sponsor_application(
    pool: web::Data<PgPool>,
    req: HttpRequest,
) -> impl Responder {
    if let Some(claims) = req.extensions().get::<Claims>() {
        // Check if application exists and get its status
        let check_query = "SELECT status FROM sponsor_applications WHERE user_id = $1";

        let result: Result<ApplicationStatus, sqlx::Error> = sqlx::query_scalar(check_query)
            .bind(&claims.id)
            .fetch_one(pool.get_ref())
            .await;

        match result {
            Ok(status) => {
                // If application is approved, user cannot delete it
                if status == ApplicationStatus::Approved {
                    return HttpResponse::Forbidden()
                        .body("You cannot delete an approved application.");
                }

                // Delete the application
                let delete_query = "DELETE FROM sponsor_applications WHERE user_id = $1";

                let result = sqlx::query(delete_query)
                    .bind(&claims.id)
                    .execute(pool.get_ref())
                    .await;

                match result {
                    Ok(_) => HttpResponse::Ok().body("Sponsor application deleted successfully."),
                    Err(_) => HttpResponse::InternalServerError()
                        .body("Failed to delete sponsor application."),
                }
            }
            Err(_) => HttpResponse::NotFound().body("No sponsor application found."),
        }
    } else {
        HttpResponse::Unauthorized().body("Authentication required")
    }
}

//Config Sponsor Routes
// POST /sponsor/apply
// GET /sponsor/application-status
// PUT /sponsor/update-application
// DELETE /sponsor/delete-application
pub fn config_sponsor_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/sponsor")
            .route("/apply", web::post().to(submit_sponsor_application))
            .route("/check", web::get().to(check_sponsor_application_status))
            .route("/update", web::patch().to(update_sponsor_application))
            .route("/delete", web::delete().to(delete_sponsor_application)),
    );
}
