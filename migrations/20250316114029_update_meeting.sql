-- Add migration script here
ALTER TABLE group_meetings
ADD COLUMN meeting_chat_id UUID REFERENCES group_chats(group_chat_id) ON DELETE SET NULL;
