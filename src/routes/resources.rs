use crate::handlers::auth::Claims;
use crate::handlers::ws;
use crate::models::all_models::{Resource, UserRole};
use actix_web::{HttpMessage, HttpRequest, HttpResponse, Responder, web};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

//Create Resource Request
#[derive(Debug, Deserialize, Serialize)]
pub struct CreateResourceRequest {
    pub title: String,
    pub content: String,
    pub support_group_id: Option<Uuid>,
}

//Update Resource Request
#[derive(Debug, Deserialize, Serialize)]
pub struct UpdateResourceRequest {
    pub title: Option<String>,
    pub content: Option<String>,
    pub support_group_id: Option<Uuid>,
}

//Create Resource
//Create Resource Input: HttpRequest(JWT Token), CreateResourceRequest
//Create Resource Output: Resource
pub async fn create_resource(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    payload: web::Json<CreateResourceRequest>,
) -> impl Responder {
    if let Some(claims) = req.extensions().get::<Claims>() {
        // Get contributor id from claims
        let contributor_id = claims.id;

        // Validate input
        if payload.title.trim().is_empty() {
            return HttpResponse::BadRequest().body("Title cannot be empty");
        }

        if payload.content.trim().is_empty() {
            return HttpResponse::BadRequest().body("Content cannot be empty");
        }

        // If support_group_id is provided, verify it exists
        if let Some(group_id) = payload.support_group_id {
            let group_exists = sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS(SELECT 1 FROM support_groups WHERE support_group_id = $1)",
            )
            .bind(group_id)
            .fetch_one(pool.get_ref())
            .await;

            match group_exists {
                Ok(exists) => {
                    if !exists {
                        return HttpResponse::BadRequest().body("Support group does not exist");
                    }
                }
                Err(e) => {
                    eprintln!("Error checking support group: {:?}", e);
                    return HttpResponse::InternalServerError()
                        .body("Error validating support group");
                }
            }
        }

        // Generate a new resource id.
        let new_resource_id = Uuid::new_v4();
        let query = "
            INSERT INTO resources (resource_id, contributor_id, title, content, approved, created_at, support_group_id)
            VALUES ($1, $2, $3, $4, false, NOW(), $5)
            RETURNING resource_id, contributor_id, title, content, approved, created_at, support_group_id
        ";
        let result = sqlx::query_as::<_, Resource>(query)
            .bind(new_resource_id)
            .bind(contributor_id)
            .bind(&payload.title)
            .bind(&payload.content)
            .bind(payload.support_group_id)
            .fetch_one(pool.get_ref())
            .await;

        match result {
            Ok(resource) => {
                // Send WebSocket notification to admins about new resource
                let notification = json!({
                    "type": "new_resource",
                    "resource_id": resource.resource_id,
                    "title": resource.title,
                    "contributor_id": resource.contributor_id,
                    "support_group_id": resource.support_group_id
                });

                // Notify admins about the new resource
                ws::send_to_role(&UserRole::Admin, notification).await;

                // If resource is associated with a support group, notify members
                if let Some(group_id) = resource.support_group_id {
                    // Get members of the support group
                    let members_query = "
                        SELECT user_id FROM support_group_members 
                        WHERE support_group_id = $1
                    ";

                    if let Ok(members) = sqlx::query_scalar::<_, Uuid>(members_query)
                        .bind(group_id)
                        .fetch_all(pool.get_ref())
                        .await
                    {
                        let group_notification = json!({
                            "type": "new_group_resource",
                            "resource_id": resource.resource_id,
                            "title": resource.title,
                            "support_group_id": group_id
                        });

                        ws::send_to_users(&members, group_notification).await;
                    }
                }

                HttpResponse::Ok().json(resource)
            }
            Err(e) => {
                eprintln!("Error creating resource: {:?}", e);
                HttpResponse::InternalServerError().body("Failed to create resource")
            }
        }
    } else {
        HttpResponse::Unauthorized().body("Authentication required")
    }
}

//Get Resource
//Get Resource Input: Path (/resources/{resource_id})
//Get Resource Output: Resource
pub async fn get_resource(pool: web::Data<PgPool>, path: web::Path<Uuid>) -> impl Responder {
    let resource_id = path.into_inner();
    let query = "
        SELECT resource_id, contributor_id, title, content, approved, created_at, support_group_id
        FROM resources WHERE resource_id = $1
    ";
    let result = sqlx::query_as::<_, Resource>(query)
        .bind(resource_id)
        .fetch_one(pool.get_ref())
        .await;

    match result {
        Ok(resource) => HttpResponse::Ok().json(resource),
        Err(e) => {
            eprintln!("Error fetching resource: {:?}", e);
            HttpResponse::NotFound().body("Resource not found")
        }
    }
}

//List Resources
//List Resources Input: None
//List Resources Output: Vec<Resource>
pub async fn list_resources(pool: web::Data<PgPool>) -> impl Responder {
    let query = "
        SELECT resource_id, contributor_id, title, content, approved, created_at, support_group_id
        FROM resources ORDER BY created_at DESC
    ";
    let result = sqlx::query_as::<_, Resource>(query)
        .fetch_all(pool.get_ref())
        .await;

    match result {
        Ok(resources) => HttpResponse::Ok().json(resources),
        Err(e) => {
            eprintln!("Error listing resources: {:?}", e);
            HttpResponse::InternalServerError().body("Failed to list resources")
        }
    }
}

//Update Resource
//Update Resource Input: HttpRequest(JWT Token), Path (/resources/{resource_id}), UpdateResourceRequest
//Update Resource Output: Resource
pub async fn update_resource(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<Uuid>,
    payload: web::Json<UpdateResourceRequest>,
) -> impl Responder {
    if let Some(claims) = req.extensions().get::<Claims>() {
        let resource_id = path.into_inner();
        let contributor_id = claims.id;

        // First check if the resource exists and belongs to the user
        let check_query =
            "SELECT resource_id FROM resources WHERE resource_id = $1 AND contributor_id = $2";
        let resource_exists = sqlx::query_scalar::<_, Uuid>(check_query)
            .bind(resource_id)
            .bind(contributor_id)
            .fetch_optional(pool.get_ref())
            .await;

        match resource_exists {
            Ok(Some(_)) => {
                // Resource exists and belongs to the user, proceed with update
                let query = "
                    UPDATE resources 
                    SET title = CASE WHEN $1::text IS NULL THEN title ELSE $1 END,
                        content = CASE WHEN $2::text IS NULL THEN content ELSE $2 END,
                        support_group_id = CASE WHEN $3::uuid IS NULL THEN support_group_id ELSE $3 END
                    WHERE resource_id = $4
                    RETURNING resource_id, contributor_id, title, content, approved, created_at, support_group_id
                ";
                let result = sqlx::query_as::<_, Resource>(query)
                    .bind(&payload.title)
                    .bind(&payload.content)
                    .bind(payload.support_group_id)
                    .bind(resource_id)
                    .fetch_one(pool.get_ref())
                    .await;

                match result {
                    Ok(updated_resource) => HttpResponse::Ok().json(updated_resource),
                    Err(e) => {
                        eprintln!("Error updating resource: {:?}", e);
                        HttpResponse::InternalServerError().body("Failed to update resource")
                    }
                }
            }
            Ok(None) => HttpResponse::NotFound().body("Resource not found or not authorized"),
            Err(e) => {
                eprintln!("Database error checking resource: {:?}", e);
                HttpResponse::InternalServerError().body("Error checking resource")
            }
        }
    } else {
        HttpResponse::Unauthorized().body("Authentication required")
    }
}

//Delete Resource
//Delete Resource Input: HttpRequest(JWT Token), Path (/resources/{resource_id})
//Delete Resource Output: Success message
pub async fn delete_resource(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<Uuid>,
) -> impl Responder {
    if let Some(claims) = req.extensions().get::<Claims>() {
        let resource_id = path.into_inner();
        let contributor_id = claims.id;

        let query = "DELETE FROM resources WHERE resource_id = $1 AND contributor_id = $2";
        let result = sqlx::query(query)
            .bind(resource_id)
            .bind(contributor_id)
            .execute(pool.get_ref())
            .await;

        match result {
            Ok(res) => {
                if res.rows_affected() > 0 {
                    HttpResponse::Ok().body("Resource deleted successfully")
                } else {
                    HttpResponse::NotFound().body("Resource not found or not authorized")
                }
            }
            Err(e) => {
                eprintln!("Error deleting resource: {:?}", e);
                HttpResponse::InternalServerError().body("Failed to delete resource")
            }
        }
    } else {
        HttpResponse::Unauthorized().body("Authentication required")
    }
}

//Config Resource Routes
// GET /resources
// GET /resources/{resource_id}
// POST /resources/new
// PATCH /resources/{resource_id}
// DELETE /resources/{resource_id}
pub fn config_resource_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/resources")
            .route("/list", web::get().to(list_resources))
            .route("/create", web::post().to(create_resource))
            .route("/{id}", web::get().to(get_resource))
            .route("/{id}", web::patch().to(update_resource))
            .route("/{id}", web::delete().to(delete_resource)),
    );
}

