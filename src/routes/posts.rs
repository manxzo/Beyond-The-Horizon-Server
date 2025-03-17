use crate::handlers::auth::Claims;
use crate::models::all_models::{Comment, Post, PostLike};
use actix_web::{web, HttpMessage, HttpRequest, HttpResponse, Responder};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{PgPool, Row};
use uuid::Uuid;

// Create Post Request
#[derive(Debug, Deserialize, Serialize)]
pub struct CreatePostRequest {
    pub content: String,
    pub tags: Option<Vec<String>>,
}

// Post with likes and comments model for API responses
#[derive(Debug, Serialize)]
pub struct PostWithDetails {
    #[serde(flatten)]
    pub post: Post,
    pub likes: Vec<PostLike>,
    pub comments: Vec<Comment>,
    pub like_count: i64,
}

// Create Post Handler
// Create Post Input: CreatePostRequest
// Create Post Output: Post
pub async fn create_post(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    payload: web::Json<CreatePostRequest>,
) -> impl Responder {
    if let Some(claims) = req.extensions().get::<Claims>() {
        let author_id = claims.id;
        let new_post_id = Uuid::new_v4();
        let query = "
            INSERT INTO posts (post_id, author_id, content, created_at, tags)
            VALUES ($1, $2, $3, NOW(), $4)
            RETURNING post_id, author_id, content, created_at, tags
        ";
        let result = sqlx::query_as::<_, Post>(query)
            .bind(new_post_id)
            .bind(author_id)
            .bind(&payload.content)
            .bind(payload.tags.clone())
            .fetch_one(pool.get_ref())
            .await;
        match result {
            Ok(post) => HttpResponse::Ok().json(post),
            Err(e) => {
                eprintln!("Error creating post: {:?}", e);
                HttpResponse::InternalServerError().body("Failed to create post")
            }
        }
    } else {
        HttpResponse::Unauthorized().body("Authentication required")
    }
}

// Get Post Handler - Returns a single post with all likes and comments
// Get Post Input: Post ID
// Get Post Output: Post with likes and comments
pub async fn get_post(pool: web::Data<PgPool>, path: web::Path<Uuid>) -> impl Responder {
    let post_id = path.into_inner();

    // Get the post
    let post_query = "
        SELECT post_id, author_id, content, created_at, tags
        FROM posts WHERE post_id = $1
    ";
    let post_result = sqlx::query_as::<_, Post>(post_query)
        .bind(post_id)
        .fetch_one(pool.get_ref())
        .await;

    match post_result {
        Ok(post) => {
            // Get likes for this post
            let likes_query = "
                SELECT post_id, user_id
                FROM post_likes
                WHERE post_id = $1
            ";
            let likes_result = sqlx::query_as::<_, PostLike>(likes_query)
                .bind(post_id)
                .fetch_all(pool.get_ref())
                .await;

            // Get comments for this post
            let comments_query = "
                SELECT comment_id, post_id, author_id, content, created_at, parent_comment_id
                FROM comments
                WHERE post_id = $1
                ORDER BY created_at ASC
            ";
            let comments_result = sqlx::query_as::<_, Comment>(comments_query)
                .bind(post_id)
                .fetch_all(pool.get_ref())
                .await;

            // Get like count
            let like_count_query = "
                SELECT COUNT(*) FROM post_likes WHERE post_id = $1
            ";
            let like_count_result = sqlx::query_scalar::<_, i64>(like_count_query)
                .bind(post_id)
                .fetch_one(pool.get_ref())
                .await;

            match (likes_result, comments_result, like_count_result) {
                (Ok(likes), Ok(comments), Ok(like_count)) => {
                    let post_with_details = PostWithDetails {
                        post,
                        likes,
                        comments,
                        like_count,
                    };
                    HttpResponse::Ok().json(post_with_details)
                }
                _ => {
                    eprintln!("Error fetching post details");
                    HttpResponse::InternalServerError().body("Failed to fetch post details")
                }
            }
        }
        Err(e) => {
            eprintln!("Error fetching post: {:?}", e);
            HttpResponse::NotFound().body("Post not found")
        }
    }
}

/// Query parameters for posts listing with pagination, tag filtering, and sorting
#[derive(Debug, Deserialize, Serialize)]
pub struct PostsListParams {
    pub page: Option<u32>,
    #[serde(rename = "search-tags")]
    pub search_tags: Option<String>,
    #[serde(rename = "sort-by")]
    pub sort_by: Option<String>, // "latest" or "most-liked"
}

// List Posts Handler with pagination, tag filtering, sorting, and includes likes and comments
// List Posts Input: Optional page, search-tags, and sort-by
// List Posts Output: List of Posts with likes, comments, and like counts (50 per page)
pub async fn list_posts(
    pool: web::Data<PgPool>,
    params: web::Query<PostsListParams>,
) -> impl Responder {
    // Default to page 1, with 50 posts per page
    let page = params.page.unwrap_or(1);
    let posts_per_page: u32 = 50;
    let offset = (page - 1) * posts_per_page;

    // Check if we need to filter by tags
    let tags: Vec<String> = match &params.search_tags {
        Some(tags_str) => tags_str.split(',').map(|s| s.trim().to_string()).collect(),
        None => Vec::new(),
    };

    // Determine sort order
    let sort_by = params.sort_by.as_deref().unwrap_or("latest");
    let order_clause = match sort_by {
        "most-liked" => "ORDER BY like_count DESC, p.created_at DESC",
        _ => "ORDER BY p.created_at DESC", // Default to latest
    };

    // Prepare response with metadata
    #[derive(Serialize)]
    struct PostsResponse {
        posts: Vec<PostWithDetails>,
        page: u32,
        posts_per_page: u32,
        total_count: i64,
    }

    // Build the base query depending on whether we're filtering by tags
    let (base_query, count_query) = if tags.is_empty() {
        // No tag filtering
        (
            format!(
                "
                WITH post_likes_count AS (
                    SELECT post_id, COUNT(*) as like_count
                    FROM post_likes
                    GROUP BY post_id
                )
                SELECT 
                    p.post_id, p.author_id, p.content, p.created_at, p.tags,
                    COALESCE(plc.like_count, 0) as like_count
                FROM posts p
                LEFT JOIN post_likes_count plc ON p.post_id = plc.post_id
                {}
                LIMIT $1 OFFSET $2
            ",
                order_clause
            ),
            "SELECT COUNT(*) FROM posts".to_string(),
        )
    } else {
        // Filter by tags
        (
            format!(
                "
                WITH post_likes_count AS (
                    SELECT post_id, COUNT(*) as like_count
                    FROM post_likes
                    GROUP BY post_id
                )
                SELECT 
                    p.post_id, p.author_id, p.content, p.created_at, p.tags,
                    COALESCE(plc.like_count, 0) as like_count
                FROM posts p
                LEFT JOIN post_likes_count plc ON p.post_id = plc.post_id
                WHERE COALESCE(p.tags, ARRAY[]::text[]) && $1::text[]
                {}
                LIMIT $2 OFFSET $3
            ",
                order_clause
            ),
            "SELECT COUNT(*) FROM posts WHERE COALESCE(tags, ARRAY[]::text[]) && $1::text[]"
                .to_string(),
        )
    };

    // Get total count for pagination metadata
    let total_count = if tags.is_empty() {
        match sqlx::query_scalar::<_, i64>(&count_query)
            .fetch_one(pool.get_ref())
            .await
        {
            Ok(count) => count,
            Err(_) => 0,
        }
    } else {
        match sqlx::query_scalar::<_, i64>(&count_query)
            .bind(&tags)
            .fetch_one(pool.get_ref())
            .await
        {
            Ok(count) => count,
            Err(_) => 0,
        }
    };

    // Execute the main query to get posts
    let posts_result = if tags.is_empty() {
        sqlx::query(&base_query)
            .bind(posts_per_page as i64)
            .bind(offset as i64)
            .fetch_all(pool.get_ref())
            .await
    } else {
        sqlx::query(&base_query)
            .bind(&tags)
            .bind(posts_per_page as i64)
            .bind(offset as i64)
            .fetch_all(pool.get_ref())
            .await
    };

    match posts_result {
        Ok(rows) => {
            let mut posts_with_details = Vec::new();

            for row in rows {
                let post_id: Uuid = row.try_get("post_id").unwrap_or_default();
                let post = Post {
                    post_id,
                    author_id: row.try_get("author_id").unwrap_or_default(),
                    content: row.try_get("content").unwrap_or_default(),
                    created_at: row.try_get("created_at").unwrap_or_default(),
                    tags: row.try_get("tags").unwrap_or_default(),
                };
                let like_count: i64 = row.try_get("like_count").unwrap_or_default();

                // Get likes for this post
                let likes_query = "
                    SELECT post_id, user_id
                    FROM post_likes
                    WHERE post_id = $1
                ";
                let likes = match sqlx::query_as::<_, PostLike>(likes_query)
                    .bind(post_id)
                    .fetch_all(pool.get_ref())
                    .await
                {
                    Ok(likes) => likes,
                    Err(e) => {
                        eprintln!("Error fetching likes for post {}: {:?}", post_id, e);
                        vec![]
                    }
                };

                // Get comments for this post
                let comments_query = "
                    SELECT comment_id, post_id, author_id, content, created_at, parent_comment_id
                    FROM comments
                    WHERE post_id = $1
                    ORDER BY created_at ASC
                ";
                let comments = match sqlx::query_as::<_, Comment>(comments_query)
                    .bind(post_id)
                    .fetch_all(pool.get_ref())
                    .await
                {
                    Ok(comments) => comments,
                    Err(e) => {
                        eprintln!("Error fetching comments for post {}: {:?}", post_id, e);
                        vec![]
                    }
                };

                posts_with_details.push(PostWithDetails {
                    post,
                    likes,
                    comments,
                    like_count,
                });
            }

            HttpResponse::Ok().json(PostsResponse {
                posts: posts_with_details,
                page,
                posts_per_page,
                total_count,
            })
        }
        Err(e) => {
            eprintln!("Error listing posts: {:?}", e);
            HttpResponse::InternalServerError().body("Failed to list posts")
        }
    }
}

// Update Post Request
#[derive(Debug, Deserialize, Serialize)]
pub struct UpdatePostRequest {
    pub content: Option<String>,
    pub tags: Option<Vec<String>>,
}

// Update Post Handler
// Update Post Input: UpdatePostRequest
// Update Post Output: Post
pub async fn update_post(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<Uuid>,
    payload: web::Json<UpdatePostRequest>,
) -> impl Responder {
    if let Some(claims) = req.extensions().get::<Claims>() {
        let post_id = path.into_inner();
        let author_id = claims.id;
        let query = "
            UPDATE posts 
            SET content = COALESCE($1, content),
                tags = COALESCE($2, tags)
            WHERE post_id = $3 AND author_id = $4
            RETURNING post_id, author_id, content, created_at, tags
        ";
        let result = sqlx::query_as::<_, Post>(query)
            .bind(&payload.content)
            .bind(payload.tags.clone())
            .bind(post_id)
            .bind(author_id)
            .fetch_one(pool.get_ref())
            .await;
        match result {
            Ok(post) => HttpResponse::Ok().json(post),
            Err(e) => {
                eprintln!("Error updating post: {:?}", e);
                HttpResponse::InternalServerError().body("Failed to update post")
            }
        }
    } else {
        HttpResponse::Unauthorized().body("Authentication required")
    }
}

// Delete Post Handler
// Delete Post Input: Post ID
// Delete Post Output: None
pub async fn delete_post(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<Uuid>,
) -> impl Responder {
    if let Some(claims) = req.extensions().get::<Claims>() {
        let post_id = path.into_inner();
        let author_id = claims.id;
        let query = "DELETE FROM posts WHERE post_id = $1 AND author_id = $2";
        let result = sqlx::query(query)
            .bind(post_id)
            .bind(author_id)
            .execute(pool.get_ref())
            .await;

        match result {
            Ok(res) => {
                if res.rows_affected() > 0 {
                    HttpResponse::Ok().body("Post deleted successfully")
                } else {
                    HttpResponse::NotFound().body("Post not found or not authorized")
                }
            }
            Err(e) => {
                eprintln!("Error deleting post: {:?}", e);
                HttpResponse::InternalServerError().body("Failed to delete post")
            }
        }
    } else {
        HttpResponse::Unauthorized().body("Authentication required")
    }
}

// Toggle Post Like Request
#[derive(Debug, Deserialize, Serialize)]
pub struct LikePostRequest {
    pub post_id: Uuid,
}

// Toggle Post Like Handler - Likes or unlikes a post
// Toggle Post Input: LikePostRequest
// Toggle Post Output: Action performed and like details
pub async fn toggle_post_like(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    payload: web::Json<LikePostRequest>,
) -> impl Responder {
    if let Some(claims) = req.extensions().get::<Claims>() {
        let user_id = claims.id;

        // First check if the user already liked the post
        let check_query =
            "SELECT EXISTS(SELECT 1 FROM post_likes WHERE post_id = $1 AND user_id = $2)";
        let already_liked = match sqlx::query_scalar::<_, bool>(check_query)
            .bind(payload.post_id)
            .bind(user_id)
            .fetch_one(pool.get_ref())
            .await
        {
            Ok(result) => result,
            Err(e) => {
                eprintln!("Error checking like status: {:?}", e);
                return HttpResponse::InternalServerError().body("Failed to check like status");
            }
        };

        if already_liked {
            // Unlike the post
            let query = "DELETE FROM post_likes WHERE post_id = $1 AND user_id = $2";
            let result = sqlx::query(query)
                .bind(payload.post_id)
                .bind(user_id)
                .execute(pool.get_ref())
                .await;

            match result {
                Ok(res) => {
                    if res.rows_affected() > 0 {
                        HttpResponse::Ok().json(json!({
                            "action": "unliked",
                            "post_id": payload.post_id
                        }))
                    } else {
                        HttpResponse::NotFound().body("Like not found or already removed")
                    }
                }
                Err(e) => {
                    eprintln!("Error unliking post: {:?}", e);
                    HttpResponse::InternalServerError().body("Failed to unlike post")
                }
            }
        } else {
            // Like the post
            let query = "
                INSERT INTO post_likes (post_id, user_id)
                VALUES ($1, $2)
                RETURNING post_id, user_id
            ";
            let result = sqlx::query_as::<_, PostLike>(query)
                .bind(payload.post_id)
                .bind(user_id)
                .fetch_one(pool.get_ref())
                .await;

            match result {
                Ok(like) => HttpResponse::Ok().json(json!({
                        "action": "liked",
                        "like": like
                })),
                Err(e) => {
                    eprintln!("Error liking post: {:?}", e);
                    HttpResponse::InternalServerError().body("Failed to like post")
                }
            }
        }
    } else {
        HttpResponse::Unauthorized().body("Authentication required")
    }
}

// Create Comment Request
#[derive(Debug, Deserialize, Serialize)]
pub struct CreateCommentRequest {
    pub post_id: Uuid,
    pub content: String,
    pub parent_comment_id: Option<Uuid>,
}

// Create Comment Handler
// Create Comment Input: CreateCommentRequest
// Create Comment Output: Comment
pub async fn create_comment(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    payload: web::Json<CreateCommentRequest>,
) -> impl Responder {
    if let Some(claims) = req.extensions().get::<Claims>() {
        let author_id = claims.id;
        let new_comment_id = Uuid::new_v4();
        let query = "
            INSERT INTO comments (comment_id, post_id, author_id, content, created_at, parent_comment_id)
            VALUES ($1, $2, $3, $4, NOW(), $5)
            RETURNING comment_id, post_id, author_id, content, created_at, parent_comment_id
        ";
        let result = sqlx::query_as::<_, Comment>(query)
            .bind(new_comment_id)
            .bind(payload.post_id)
            .bind(author_id)
            .bind(&payload.content)
            .bind(payload.parent_comment_id)
            .fetch_one(pool.get_ref())
            .await;
        match result {
            Ok(comment) => HttpResponse::Ok().json(comment),
            Err(e) => {
                eprintln!("Error creating comment: {:?}", e);
                HttpResponse::InternalServerError().body("Failed to create comment")
            }
        }
    } else {
        HttpResponse::Unauthorized().body("Authentication required")
    }
}

// Update Comment Request
#[derive(Debug, Deserialize, Serialize)]
pub struct UpdateCommentRequest {
    pub content: Option<String>,
}

// Update Comment Handler
// Update Comment Input: UpdateCommentRequest
// Update Comment Output: Comment
pub async fn update_comment(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<Uuid>, // comment_id
    payload: web::Json<UpdateCommentRequest>,
) -> impl Responder {
    if let Some(claims) = req.extensions().get::<Claims>() {
        let comment_id = path.into_inner();
        let author_id = claims.id;
        let query = "
            UPDATE comments
            SET content = COALESCE($1, content)
            WHERE comment_id = $2 AND author_id = $3
            RETURNING comment_id, post_id, author_id, content, created_at, parent_comment_id
        ";
        let result = sqlx::query_as::<_, Comment>(query)
            .bind(&payload.content)
            .bind(comment_id)
            .bind(author_id)
            .fetch_one(pool.get_ref())
            .await;
        match result {
            Ok(comment) => HttpResponse::Ok().json(comment),
            Err(e) => {
                eprintln!("Error updating comment: {:?}", e);
                HttpResponse::InternalServerError().body("Failed to update comment")
            }
        }
    } else {
        HttpResponse::Unauthorized().body("Authentication required")
    }
}

// Delete Comment Handler
// Delete Comment Input: Comment ID
// Delete Comment Output: String
pub async fn delete_comment(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<Uuid>, // comment_id
) -> impl Responder {
    if let Some(claims) = req.extensions().get::<Claims>() {
        let comment_id = path.into_inner();
        let author_id = claims.id;

        // Now delete the comment
        let query = "DELETE FROM comments WHERE comment_id = $1 AND author_id = $2";
        let result = sqlx::query(query)
            .bind(comment_id)
            .bind(author_id)
            .execute(pool.get_ref())
            .await;

        match result {
            Ok(res) => {
                if res.rows_affected() > 0 {
                    HttpResponse::Ok().body("Comment deleted successfully")
                } else {
                    HttpResponse::NotFound().body("Comment not found or not authorized")
                }
            }
            Err(e) => {
                eprintln!("Error deleting comment: {:?}", e);
                HttpResponse::InternalServerError().body("Failed to delete comment")
            }
        }
    } else {
        HttpResponse::Unauthorized().body("Authentication required")
    }
}

// Feed Routes
// GET /feed/posts - List posts with pagination and optional tag filtering
// POST /feed/posts/new - Create a new post
// GET /feed/posts/{id} - Get a specific post
// PATCH /feed/posts/{id} - Update a post
// DELETE /feed/posts/{id} - Delete a post
// POST /feed/posts/like - Like a post
// POST /feed/comments - Create a comment
// GET /feed/posts/{post_id}/comments - List comments for a post
// PATCH /feed/comments/{id} - Update a comment
// DELETE /feed/comments/{id} - Delete a comment
pub fn config_feed_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/feed")
            // Post routes
            .route("/posts", web::get().to(list_posts))
            .route("/posts/new", web::post().to(create_post))
            .route("/posts/{id}", web::get().to(get_post))
            .route("/posts/{id}", web::patch().to(update_post))
            .route("/posts/{id}", web::delete().to(delete_post))
            // Like routes
            .route("/posts/like", web::post().to(toggle_post_like))
            // Comment routes
            .route("/comments", web::post().to(create_comment))
            .route("/comments/{id}", web::patch().to(update_comment))
            .route("/comments/{id}", web::delete().to(delete_comment)),
    );
}
