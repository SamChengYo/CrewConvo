-- People sign up with an e-mail address, a name and a password. Accounts created before this
-- have no e-mail and keep signing in with their name.
ALTER TABLE human_credentials ADD COLUMN email text;
CREATE UNIQUE INDEX human_credentials_email_key ON human_credentials (lower(email)) WHERE email IS NOT NULL;
