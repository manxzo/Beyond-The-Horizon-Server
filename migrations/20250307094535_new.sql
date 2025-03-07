-- Add migration script here
-- ENUM TYPES
CREATE TYPE user_role AS ENUM ('member', 'sponsor', 'admin');
CREATE TYPE application_status AS ENUM ('pending', 'approved', 'rejected');
CREATE TYPE matching_status AS ENUM ('pending', 'accepted', 'declined');
CREATE TYPE meeting_status AS ENUM ('upcoming', 'ongoing', 'ended');
CREATE TYPE support_group_status AS ENUM ('pending', 'approved', 'rejected');
CREATE TYPE reported_type AS ENUM ('message', 'groupchatmessage', 'groupchat', 'user', 'post', 'comment');
CREATE TYPE report_status AS ENUM ('pending', 'resolved', 'reviewed');
CREATE TYPE announcement_type AS ENUM (
  'general',
  'newsponsorapplication',
  'sponsorapplicationapproved',
  'sponsorapplicationrejected',
  'supportgroupsuggestion',
  'supportgroupapproved',
  'supportgrouprejected',
  'meetingscheduled',
  'meetingreminder',
  'meetingstarted',
  'meetingended',
  'groupchatinvitation',
  'privatechatinvitation',
  'newpost',
  'newcomment',
  'postlike',
  'commentreply',
  'newresource',
  'matchingrequestsubmitted',
  'matchingrequestaccepted',
  'matchingrequestdeclined',
  'adminaction'
);
CREATE TYPE announcement_target AS ENUM (
  'user',
  'sponsorapplication',
  'supportgroup',
  'groupmeeting',
  'groupchat',
  'chat',
  'post',
  'comment',
  'resource',
  'matchingrequest'
);

-- USERS TABLE
CREATE TABLE users (
    user_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    username TEXT NOT NULL UNIQUE CHECK (char_length(username) >= 3),
    email TEXT NOT NULL UNIQUE CHECK (position('@' IN email) > 1),
    password_hash TEXT NOT NULL,
    role user_role NOT NULL DEFAULT 'member',
    banned_until TIMESTAMP,
    avatar_url TEXT NOT NULL CHECK (char_length(avatar_url) > 5),
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    dob DATE NOT NULL CHECK (dob <= NOW() - INTERVAL '16 years'),
    user_profile TEXT NOT NULL CHECK (char_length(user_profile) > 10),
    bio TEXT,
    email_verified BOOLEAN NOT NULL DEFAULT FALSE,
    email_verification_token UUID,
    forgot_password_token UUID,
    forgot_password_expires_at TIMESTAMP,
    location JSONB,
    interests TEXT[],
    experience TEXT[],
    available_days TEXT[],
    languages TEXT[],
    privacy BOOLEAN NOT NULL DEFAULT FALSE
);

-- SPONSOR APPLICATIONS TABLE
CREATE TABLE sponsor_applications (
    application_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    status application_status NOT NULL DEFAULT 'pending',
    application_info TEXT NOT NULL CHECK (char_length(application_info) > 20),
    reviewed_by UUID REFERENCES users(user_id) ON DELETE SET NULL,
    admin_comments TEXT,
    created_at TIMESTAMP NOT NULL DEFAULT NOW()
);

-- MATCHING REQUESTS TABLE
CREATE TABLE matching_requests (
    matching_request_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    member_id UUID NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    sponsor_id UUID REFERENCES users(user_id) ON DELETE SET NULL,
    status matching_status NOT NULL DEFAULT 'pending',
    match_score REAL CHECK (match_score >= 0 AND match_score <= 100),
    created_at TIMESTAMP NOT NULL DEFAULT NOW()
);

-- MATCH USERS TABLE (for matching algorithm)
CREATE TABLE match_users (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    dob DATE NOT NULL,
    location JSONB,
    interests TEXT[],
    experience TEXT[],
    available_days TEXT[],
    languages TEXT[]
);

-- MESSAGES TABLE
CREATE TABLE messages (
    message_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    sender_id UUID NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    receiver_id UUID NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    content TEXT NOT NULL CHECK (char_length(content) > 1),
    timestamp TIMESTAMP NOT NULL DEFAULT NOW(),
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    edited BOOLEAN NOT NULL DEFAULT FALSE,
    seen_at TIMESTAMP
);

-- GROUP CHATS TABLE (moved up)
CREATE TABLE group_chats (
    group_chat_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    creator_id UUID NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    flagged BOOLEAN DEFAULT FALSE
);

-- GROUP CHAT MEMBERS TABLE
CREATE TABLE group_chat_members (
    group_chat_id UUID NOT NULL REFERENCES group_chats(group_chat_id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    PRIMARY KEY (group_chat_id, user_id)
);

-- GROUP CHAT MESSAGES TABLE
CREATE TABLE group_chat_messages (
    group_chat_message_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    group_chat_id UUID NOT NULL REFERENCES group_chats(group_chat_id) ON DELETE CASCADE,
    sender_id UUID NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    content TEXT NOT NULL CHECK (char_length(content) > 1),
    timestamp TIMESTAMP NOT NULL DEFAULT NOW(),
    deleted BOOLEAN NOT NULL DEFAULT FALSE,
    edited BOOLEAN NOT NULL DEFAULT FALSE
);

-- SUPPORT GROUPS TABLE (moved up)
CREATE TABLE support_groups (
    support_group_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    title TEXT NOT NULL,
    description TEXT NOT NULL,
    admin_id UUID REFERENCES users(user_id) ON DELETE SET NULL,
    group_chat_id UUID REFERENCES group_chats(group_chat_id) ON DELETE SET NULL,
    status support_group_status NOT NULL DEFAULT 'pending',
    created_at TIMESTAMP NOT NULL DEFAULT NOW()
);

-- SUPPORT GROUP MEMBERS TABLE
CREATE TABLE support_group_members (
    support_group_id UUID NOT NULL REFERENCES support_groups(support_group_id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    joined_at TIMESTAMP NOT NULL DEFAULT NOW(),
    PRIMARY KEY (support_group_id, user_id)
);

-- GROUP MEETINGS TABLE
CREATE TABLE group_meetings (
    meeting_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    group_chat_id UUID REFERENCES group_chats(group_chat_id) ON DELETE SET NULL,
    support_group_id UUID NOT NULL REFERENCES support_groups(support_group_id) ON DELETE CASCADE,
    host_id UUID NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    title TEXT NOT NULL CHECK (char_length(title) >= 5),
    description TEXT,
    scheduled_time TIMESTAMP NOT NULL,
    status meeting_status NOT NULL
);

-- MEETING PARTICIPANTS TABLE
CREATE TABLE meeting_participants (
    meeting_id UUID NOT NULL REFERENCES group_meetings(meeting_id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    PRIMARY KEY (meeting_id, user_id)
);

-- RESOURCES TABLE
CREATE TABLE resources (
    resource_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    contributor_id UUID NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    title TEXT NOT NULL CHECK (char_length(title) >= 5),
    content TEXT NOT NULL CHECK (char_length(content) > 20),
    approved BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    support_group_id UUID REFERENCES support_groups(support_group_id) ON DELETE SET NULL
);

-- REPORTS TABLE
CREATE TABLE reports (
    report_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    reporter_id UUID NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    reported_user_id UUID REFERENCES users(user_id) ON DELETE CASCADE,
    reason TEXT NOT NULL CHECK (char_length(reason) >= 10),
    reported_type reported_type NOT NULL,
    reported_item_id UUID NOT NULL,
    status report_status NOT NULL DEFAULT 'pending',
    reviewed_by UUID REFERENCES users(user_id) ON DELETE SET NULL,
    resolved_at TIMESTAMP,
    created_at TIMESTAMP NOT NULL DEFAULT NOW()
);

-- POSTS TABLE
CREATE TABLE posts (
    post_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    author_id UUID NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    content TEXT NOT NULL CHECK (char_length(content) > 5),
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    tags TEXT[]
);

-- POST LIKES TABLE
CREATE TABLE post_likes (
    post_id UUID NOT NULL REFERENCES posts(post_id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    PRIMARY KEY (post_id, user_id)
);

-- COMMENTS TABLE
CREATE TABLE comments (
    comment_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    post_id UUID NOT NULL REFERENCES posts(post_id) ON DELETE CASCADE,
    author_id UUID NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
    content TEXT NOT NULL CHECK (char_length(content) > 1),
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    parent_comment_id UUID REFERENCES comments(comment_id) ON DELETE SET NULL
);

-- ANNOUNCEMENTS TABLE
CREATE TABLE announcements (
    announcement_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    announcement_type announcement_type NOT NULL,
    announcement_target announcement_target,
    announcement_target_id UUID,
    recipient_role user_role,
    recipient_id UUID,
    extra_data JSONB,
    message TEXT NOT NULL CHECK (char_length(message) > 5),
    created_at TIMESTAMP NOT NULL DEFAULT NOW()
);
