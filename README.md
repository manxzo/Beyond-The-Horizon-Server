# Beyond The Horizon - Server

![logo](/icon.png)
Beyond The Horizon is my first Rust project, built as I transitioned from JavaScript to Rust development. It's a robust server application powering a support group and community platform. While the learning curve was steep, the journey taught me invaluable lessons about systems programming, memory management, and building high-performance web services.

### Live Page URL [Beyond The Horizon](beyondthehorizon.my)

### Link to Client Repository [Beyond-The-Horizon-Client](https://github.com/manxzo/Beyond-The-Horizon-Client)

### Live Server Shuttle URL [Beyond The Horizon Server](https://bth-server-ywjx.shuttle.app/)

---

## Table of Contents

- [Overview](#overview)
- [Route Structure](#route-structure)
- [My Learning Journey & Challenges](#my-learning-journey--challenges)
- [AI Assistance & Implementation](#ai-assistance--implementation)
- [Database Schema](#database-schema)
- [Security Implementation](#security-implementation)
- [Performance Features](#performance-features)
- [Dependencies](#dependencies)
- [Development Challenges](#development-challenges)
- [References](#references)

---

## Overview

Beyond The Horizon's server component is designed to support a platform where:

- Users can create and join support groups
- Members can connect with experienced sponsors
- Groups can organize and manage meetings
- Users can share and access resources
- Administrators can effectively moderate content
- Real-time communication enables immediate support

### Key Features

- Secure user authentication and authorization
- Real-time WebSocket communication
- File storage with B2 Cloud integration
- Location-based matching system
- Support group management
- Resource sharing platform
- Administrative tools

---

## Route Structure

The server is organized into distinct route modules, each handling specific functionality:

### User Management Routes

```rust
// user_auth.rs
pub async fn create_user(
    pool: web::Data<PgPool>,
    payload: web::Json<CreateUserRequest>
) -> impl Responder {
    // Example request:
    {
        "username": "john_doe",
        "email": "john@example.com",
        "password": "secure_password",
        "dob": "1990-01-01"
    }
    // Returns: CreatedUserResponse
}

// user_data.rs
pub async fn update_user_profile(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    payload: web::Json<UpdateUserRequest>
) -> impl Responder {
    // Example request:
    {
        "user_profile": "About me...",
        "bio": "My bio...",
        "location": {"city": "New York"},
        "interests": ["anxiety", "depression"],
        "experience": ["5 years recovery"],
        "available_days": ["Monday", "Wednesday"],
        "languages": ["English", "Spanish"],
        "privacy": true
    }
    // Returns: UpdatedUserProfile
}
```

### Support Group Routes

```rust
// support_groups.rs
pub async fn suggest_support_group(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    payload: web::Json<SuggestSupportGroupRequest>
) -> impl Responder {
    // Example request:
    {
        "title": "Anxiety Support Group",
        "description": "A safe space for anxiety discussion"
    }
    // Returns: SupportGroup
}

// support_group_meetings.rs
pub async fn create_support_group_meeting(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    payload: web::Json<CreateSupportGroupMeetingRequest>
) -> impl Responder {
    // Example request:
    {
        "support_group_id": "uuid",
        "title": "Weekly Meeting",
        "description": "Our weekly support session",
        "scheduled_time": "2024-01-20T15:00:00"
    }
    // Returns: GroupMeeting
}
```

### Communication Routes

```rust
// private_messaging.rs
pub async fn send_message(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    payload: web::Json<SendMessageRequest>
) -> impl Responder {
    // Example request:
    {
        "receiver_username": "jane_doe",
        "content": "Hello, how are you?"
    }
    // Returns: Message
}

// group_chats.rs
pub async fn send_group_chat_message(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    path: web::Path<Uuid>,
    payload: web::Json<SendGroupChatMessageRequest>
) -> impl Responder {
    // Example request:
    {
        "content": "Hello everyone!"
    }
    // Returns: GroupChatMessage
}
```

### Content Management Routes

```rust
// posts.rs
pub async fn create_post(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    payload: web::Json<CreatePostRequest>
) -> impl Responder {
    // Example request:
    {
        "content": "Sharing my recovery journey...",
        "tags": ["recovery", "motivation"]
    }
    // Returns: Post
}

// resources.rs
pub async fn create_resource(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    payload: web::Json<CreateResourceRequest>
) -> impl Responder {
    // Example request:
    {
        "title": "Coping Strategies",
        "content": "Useful techniques for anxiety...",
        "support_group_id": "optional-uuid"
    }
    // Returns: Resource
}
```

### Sponsor System Routes

```rust
// sponsor_role.rs (227 lines)
pub async fn apply_for_sponsor(user: AuthenticatedUser, data: web::Json<ApplicationData>) -> Result<HttpResponse, Error> {
    // Sponsor application logic
}

// sponsor_matching.rs (375 lines)
pub async fn match_with_sponsor(user: AuthenticatedUser, data: web::Json<MatchingData>) -> Result<HttpResponse, Error> {
    // Sponsor matching logic
}
```

### Administration Routes

```rust
// admin.rs
pub async fn review_sponsor_application(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    payload: web::Json<ReviewSponsorApplicationRequest>
) -> impl Responder {
    // Example request:
    {
        "application_id": "uuid",
        "status": "approved",
        "admin_comments": "Application approved..."
    }
    // Returns: AdminActionResponse
}

// report.rs
pub async fn create_report(
    pool: web::Data<PgPool>,
    req: HttpRequest,
    payload: web::Json<CreateReportRequest>
) -> impl Responder {
    // Example request:
    {
        "reported_user_id": "uuid",
        "reason": "Inappropriate behavior",
        "reported_type": "Message",
        "reported_item_id": "uuid"
    }
    // Returns: Report
}
```

---

## AI Assistance & Implementation

### Project Structure & Migration

The migration to Shuttle was implemented using official Shuttle templates and documentation:

```rust
// Shuttle-based main.rs using official template
#[shuttle_runtime::main]
async fn actix_web(
    #[shuttle_shared_db::Postgres] pool: PgPool,
) -> shuttle_actix_web::ShuttleActixWeb<impl FnOnce(&mut ServiceConfig) + Send + Clone + 'static> {
    let pool = setup_database(pool).await?;

    let config = move |cfg: &mut ServiceConfig| {
        cfg.app_data(web::Data::new(pool.clone()))
           .configure(configure_routes)
           .wrap(setup_cors());
    };

    Ok(config.into())
}
```

The project structure follows Shuttle's recommended patterns for:

- Database configuration
- Route organization
- CORS setup
- Environment management
- Deployment workflow

### Error Handling Templates

AI helped create consistent error handling patterns:

```rust
#[derive(Debug, Error)]
pub enum AppError {
    #[error("Authentication failed: {0}")]
    AuthError(String),

    #[error("Database error: {0}")]
    DatabaseError(#[from] sqlx::Error),

    #[error("Validation error: {0}")]
    ValidationError(String),

    #[error("Not found: {0}")]
    NotFound(String),
}

impl ResponseError for AppError {
    fn error_response(&self) -> HttpResponse {
        match self {
            AppError::AuthError(_) => HttpResponse::Unauthorized(),
            AppError::DatabaseError(_) => HttpResponse::InternalServerError(),
            AppError::ValidationError(_) => HttpResponse::BadRequest(),
            AppError::NotFound(_) => HttpResponse::NotFound(),
        }
        .json(json!({
            "error": self.to_string()
        }))
    }
}
```

## My Learning Journey & Challenges

Coming from a JavaScript background, learning Rust was both exciting and challenging. Here's my personal journey:

### The Memory Management Learning Curve

The biggest initial hurdle was understanding Rust's ownership system - it was a complete paradigm shift from JavaScript's garbage collection. Some key challenges I faced:

- **Understanding Ownership**: Coming from JavaScript where memory management is automatic, I struggled with Rust's borrowing rules. I spent considerable time learning about:

  - Why we can't have multiple mutable references
  - How to properly use references vs owned values
  - When to use Clone vs borrowing
  - Understanding lifetime annotations

- **Common Pitfalls I Encountered**:

  ```rust
  // This would fail - multiple mutable borrows
  let mut data = vec![1, 2, 3];
  let ref1 = &mut data;
  let ref2 = &mut data; // Error!

  // Learning to fix it with scopes
  {
      let ref1 = &mut data;
      // work with ref1
  } // ref1 goes out of scope
  let ref2 = &mut data; // Now this works
  ```

### Wrestling with the Type System

The strict type system was another significant challenge. Some areas where I needed extra learning:

- **Error Handling**: Learning to use Result and Option types effectively
- **Custom Error Types**: Creating my own error types for better error handling
- **Trait Implementation**: Understanding how to implement traits for custom types

Here's an example of a custom error type I created (with AI assistance initially):

```rust
#[derive(Debug)]
pub enum AppError {
    AuthError(String),
    DatabaseError(sqlx::Error),
    ValidationError(String),
}

impl std::error::Error for AppError {}

impl From<sqlx::Error> for AppError {
    fn from(err: sqlx::Error) -> Self {
        AppError::DatabaseError(err)
    }
}
```

### Async Programming Adventures

Learning async/await in Rust was different from JavaScript's Promise system. Key challenges included:

- Understanding the Tokio runtime
- Dealing with Future composition
- Handling async traits (which required special attention)
- Managing error handling in async contexts

## AI Assistance & Learning

Throughout this project, I used AI as a learning tool in several key areas:

### Middleware Implementation

AI helped me understand and initially implement middleware patterns. Here's an example of middleware I learned to write:

```rust
pub struct AuthMiddleware;

impl AuthMiddleware {
    async fn validate_token(&self, token: &str) -> Result<Claims, Error> {
        // Initially AI-generated, then understood and modified by me
        let key = std::env::var("JWT_SECRET")?;
        let validation = Validation::new(Algorithm::HS256);
        let token_data = decode::<Claims>(token, key.as_bytes(), &validation)?;
        Ok(token_data.claims)
    }
}
```

### Database Queries

For complex SQL queries, I used AI to:

- Generate initial query structures
- Understand SQLx macro usage
- Learn about proper indexing

Example of a complex query I learned to write:

```rust
pub async fn get_user_groups(
    pool: &PgPool,
    user_id: Uuid,
) -> Result<Vec<Group>, sqlx::Error> {
    sqlx::query_as!(
        Group,
        r#"
        SELECT g.*
        FROM groups g
        INNER JOIN group_members gm ON g.id = gm.group_id
        WHERE gm.user_id = $1
        "#,
        user_id
    )
    .fetch_all(pool)
    .await
}
```

### WebSocket Implementation

The WebSocket handler was particularly challenging. AI helped me understand:

- Connection management
- Message broadcasting
- Room management
- Error recovery patterns

## Development Process & Learnings

### Database Design Evolution

My database schema went through several iterations as I learned more about:

- Proper normalization
- Index optimization
- Relationship modeling
- Performance considerations

### Authentication System Development

Building the authentication system taught me about:

- JWT implementation in Rust
- Proper password hashing with Argon2
- Session management
- Security best practices

### Real-time Features

Implementing real-time features was challenging but rewarding:

- WebSocket management in Rust
- Handling concurrent connections
- Managing shared state
- Implementing heartbeat mechanisms

## Code Examples & Learning Moments

### Error Handling Evolution

My error handling improved significantly over time:

```rust
// Early version (basic)
fn handle_error(err: Error) -> Response {
    Response::error(err.to_string())
}

// Later version (more sophisticated)
pub async fn handle_error(err: AppError) -> impl Responder {
    match err {
        AppError::AuthError(msg) => HttpResponse::Unauthorized().json(json!({
            "error": "Authentication failed",
            "message": msg
        })),
        AppError::DatabaseError(e) => {
            log::error!("Database error: {:?}", e);
            HttpResponse::InternalServerError().json(json!({
                "error": "Internal server error"
            }))
        },
        // ... more error cases
    }
}
```

## References & Learning Resources

1. [Rust Book](https://doc.rust-lang.org/book/) - My primary learning resource
2. [Actix Web Examples](https://github.com/actix/examples) - Helped with web framework understanding
3. [SQLx Documentation](https://docs.rs/sqlx/) - Essential for database work
4. [Tokio Tutorials](https://tokio.rs/tokio/tutorial) - Crucial for async programming
5. [JWT Authentication Guide](https://jwt.io/introduction) - Authentication implementation
6. [Rust by Example](https://doc.rust-lang.org/rust-by-example/) - Practical learning
7. [Rust Cookbook](https://rust-lang-nursery.github.io/rust-cookbook/) - Common programming solutions

## AI Assistance Note

This project was developed with strategic use of AI tools to:

- Generate initial templates for complex patterns
- Debug challenging issues
- Understand Rust-specific concepts
- Implement security features
- Optimize database queries
- Structure the application architecture

While AI was helpful in generating boilerplate code and explaining complex concepts, I made sure to:

- Understand every piece of code before implementing it
- Modify and adapt AI-generated code to fit my specific needs
- Learn the underlying concepts thoroughly
- Write critical business logic myself
- Maintain code quality and security standards

The goal was to use AI as a learning accelerator while ensuring I fully understood and could maintain every aspect of the codebase.

---

## Database Schema

### User Management

```sql
Table: users
- user_id: UUID (Primary Key)
- username: String
- email: String (Unique)
- password_hash: String
- role: Enum(Member, Sponsor, Admin)
- banned_until: Optional<DateTime>
- created_at: DateTime
- email_verified: Boolean
```

### Support Groups

```sql
Table: support_groups
- group_id: UUID (Primary Key)
- name: String
- description: Text
- creator_id: UUID (Foreign Key -> users)
- created_at: DateTime
- status: Enum(Active, Archived)
- privacy: Boolean
```

### Meetings

```sql
Table: meetings
- meeting_id: UUID (Primary Key)
- group_id: UUID (Foreign Key -> support_groups)
- title: String
- description: Text
- scheduled_for: DateTime
- location: JsonB
- meeting_type: Enum(InPerson, Virtual)
```

### Resources

```sql
Table: resources
- resource_id: UUID (Primary Key)
- title: String
- description: Text
- url: Optional<String>
- file_path: Optional<String>
- created_by: UUID (Foreign Key -> users)
- created_at: DateTime
```

---

## API Routes & Endpoints

### Authentication Routes (`user_auth.rs`)

```rust
// Public Routes
POST    /api/public/auth/register           // Register new user
POST    /api/public/auth/login              // User login
POST    /api/public/auth/refresh            // Refresh JWT token
GET     /api/public/auth/verify-email       // Verify email address
POST    /api/public/auth/reset-password     // Reset password
POST    /api/public/auth/request-reset      // Request password reset

// Protected Routes
POST    /api/protected/auth/logout          // User logout
POST    /api/protected/auth/change-password // Change password
```

### User Data Routes (`user_data.rs`)

```rust
GET     /api/protected/users/info           // Get current user info
GET     /api/protected/users/{username}     // Get user by username
GET     /api/protected/users/id/{user_id}   // Get user by ID
PATCH   /api/protected/users/update-info    // Update user profile
POST    /api/protected/users/avatar/upload  // Upload avatar
POST    /api/protected/users/avatar/reset   // Reset avatar
DELETE  /api/protected/users/delete-user    // Delete account
```

### Support Group Routes (`support_groups.rs`)

```rust
POST    /api/protected/support-groups/suggest    // Suggest new group
GET     /api/protected/support-groups/list       // List all groups
GET     /api/protected/support-groups/{group_id} // Get group details
POST    /api/protected/support-groups/join       // Join group
DELETE  /api/protected/support-groups/{id}/leave // Leave group
GET     /api/protected/support-groups/my         // Get user's groups
```

### Meeting Routes (`support_group_meetings.rs`)

```rust
POST    /api/protected/meetings/new              // Create meeting
GET     /api/protected/meetings/{meeting_id}     // Get meeting details
GET     /api/protected/meetings/user             // Get user's meetings
POST    /api/protected/meetings/join             // Join meeting
DELETE  /api/protected/meetings/{id}/leave       // Leave meeting
GET     /api/protected/meetings/{id}/participants// Get participants
POST    /api/protected/meetings/{id}/start       // Start meeting
POST    /api/protected/meetings/{id}/end         // End meeting
```

### Private Messaging Routes (`private_messaging.rs`)

```rust
GET     /api/protected/messages/conversations    // Get conversations
GET     /api/protected/messages/conversation/{username} // Get messages with user
POST    /api/protected/messages/send            // Send message
PUT     /api/protected/messages/{id}/seen       // Mark as seen
PUT     /api/protected/messages/{id}/edit       // Edit message
DELETE  /api/protected/messages/{id}            // Delete message
POST    /api/protected/messages/{id}/report     // Report message
```

### Group Chat Routes (`group_chats.rs`)

```rust
POST    /api/protected/group-chats/create       // Create group chat
GET     /api/protected/group-chats/list         // List group chats
GET     /api/protected/group-chats/{id}         // Get chat details
POST    /api/protected/group-chats/{id}/messages// Send group message
PATCH   /api/protected/group-chats/{id}/messages/{msg_id} // Edit message
DELETE  /api/protected/group-chats/{id}/messages/{msg_id} // Delete message
POST    /api/protected/group-chats/{id}/members // Add member
DELETE  /api/protected/group-chats/{id}/members/{user_id} // Remove member
```

### Posts Routes (`posts.rs`)

```rust
GET     /api/protected/feed/posts               // Get feed posts
GET     /api/protected/feed/posts/{id}          // Get specific post
POST    /api/protected/feed/posts/new           // Create post
PATCH   /api/protected/feed/posts/{id}          // Update post
DELETE  /api/protected/feed/posts/{id}          // Delete post
POST    /api/protected/feed/posts/like          // Like post
POST    /api/protected/feed/comments            // Add comment
PATCH   /api/protected/feed/comments/{id}       // Edit comment
DELETE  /api/protected/feed/comments/{id}       // Delete comment
```

### Resource Routes (`resources.rs`)

```rust
GET     /api/protected/resources/list           // List resources
GET     /api/protected/resources/{id}           // Get resource
POST    /api/protected/resources/create         // Create resource
PATCH   /api/protected/resources/{id}           // Update resource
DELETE  /api/protected/resources/{id}           // Delete resource
```

### Sponsor System Routes (`sponsor_role.rs`, `sponsor_matching.rs`)

```rust
// Sponsor Role
POST    /api/protected/sponsor/apply            // Apply for sponsor
GET     /api/protected/sponsor/check            // Check application status
PATCH   /api/protected/sponsor/update           // Update application
DELETE  /api/protected/sponsor/delete           // Delete application

// Sponsor Matching
GET     /api/protected/matching/recommend-sponsors // Get recommendations
POST    /api/protected/matching/request-sponsor    // Request sponsor
GET     /api/protected/matching/status            // Get matching status
PATCH   /api/protected/matching/respond           // Respond to request
```

### Admin Routes (`admin.rs`)

```rust
// Sponsor Applications
GET     /api/protected/admin/sponsor-applications/pending // Get pending
POST    /api/protected/admin/sponsor-applications/review  // Review application

// Support Groups
GET     /api/protected/admin/support-groups/pending      // Get pending
POST    /api/protected/admin/support-groups/review       // Review group

// Resources
GET     /api/protected/admin/resources/pending          // Get pending
POST    /api/protected/admin/resources/review           // Review resource

// Reports
GET     /api/protected/admin/reports/unresolved         // Get unresolved
POST    /api/protected/admin/reports/handle             // Handle report

// User Management
POST    /api/protected/admin/users/ban                  // Ban user
POST    /api/protected/admin/users/unban                // Unban user
GET     /api/protected/admin/users/banned               // Get banned users
GET     /api/protected/admin/users                      // Get all users
GET     /api/protected/admin/stats                      // Get admin stats
```

### Report Routes (`report.rs`)

```rust
POST    /api/protected/reports/new                      // Create report
```

Each route includes proper authentication middleware and error handling as shown in the implementation sections above. All protected routes require a valid JWT token and appropriate user permissions.

---

## Core Components

### Middleware Implementation

1. **Authentication Middleware**

```rust
pub struct AuthMiddleware;

impl AuthMiddleware {
    async fn validate_token(&self, token: &str) -> Result<Claims, Error> {
        // Token validation logic
    }

    async fn check_permissions(&self, claims: &Claims) -> Result<bool, Error> {
        // Permission checking logic
    }
}
```

2. **Request Logger**

- Structured logging implementation
- Performance metrics tracking
- Error monitoring
- Rate limiting

3. **Session Management**

- Token refresh handling
- Session state tracking
- Security enforcement
- Concurrent session management

### Handlers

1. **WebSocket Handler**

- Connection pooling
- Message broadcasting
- Room management
- State tracking
- Error recovery

2. **File Storage Handler**

- B2 Cloud Storage integration
- Chunked uploads
- File validation
- Metadata management

3. **Authentication Handler**

- Password hashing (Argon2)
- JWT management
- Session handling
- Security enforcement

---

## Security Implementation

### Authentication

- Argon2 password hashing
- JWT with key rotation
- Session management
- MFA support
- Account recovery

### Data Protection

- Input validation
- SQL injection prevention
- XSS protection
- CSRF protection
- Rate limiting

### Access Control

- Role-based access
- Resource permissions
- API protection
- Data isolation
- Audit logging

---

## Performance Features

### Database Optimization

- Connection pooling
- Query optimization
- Index management
- Caching strategies
- Async operations

### API Performance

- Response caching
- Compression
- Rate limiting
- Batch operations
- Connection pooling

---

## Dependencies

```toml
[dependencies]
actix-web = "4.9.0"
actix-multipart = "0.7.2"
actix-identity = "0.8.0"
actix-session = { version = "0.10.1", features = ["cookie-session"] }
actix-cors = "0.7.0"
sqlx = { version = "0.8.3", features = ["runtime-tokio-native-tls", "postgres", "macros","chrono","uuid"] }
tokio = { version = "1.44.0", features = ["full"] }
jsonwebtoken = "9.3.1"
chrono = { version = "0.4.40", features = ["serde"] }
argon2 = "0.5.3"
shuttle-runtime = "0.52.0"
shuttle-actix-web = "0.52.0"
shuttle-shared-db = { version = "0.52.0", features = ["postgres"] }
```

---

## Development Challenges

### Database Design

- **Challenge:** Complex entity relationships
- **Solution:**
  - Normalized schema design
  - Efficient indexing
  - Query optimization
  - Constraint management
  - Performance monitoring

### Real-time Communication

- **Challenge:** Scalable WebSocket system
- **Solution:**
  - Custom connection pool
  - Message queue integration
  - Heart-beat mechanism
  - Auto-reconnection
  - Load balancing

### File Storage

- **Challenge:** Secure file handling
- **Solution:**
  - Chunked uploads
  - Content validation
  - Metadata management
  - Caching strategy
  - Cleanup routines

### Authentication

- **Challenge:** Secure user management
- **Solution:**
  - Multi-layer security
  - Session management
  - Rate limiting
  - Audit logging
  - Recovery processes

---

## References

1. [Rust Documentation](https://doc.rust-lang.org/book/)
2. [Actix Web Framework](https://actix.rs/)
3. [SQLx Documentation](https://docs.rs/sqlx/)
4. [Tokio Async Runtime](https://tokio.rs/)
5. [JWT Authentication](https://jwt.io/)
6. [Backblaze B2 API](https://www.backblaze.com/b2/docs/)

---

## AI Assistance Note

This project was developed with AI assistance for:

- Understanding Rust concepts
- Debugging complex issues
- Security implementation
- Query optimization
- Architecture design

Most code was written and understood by me, with AI serving as a learning aid.
