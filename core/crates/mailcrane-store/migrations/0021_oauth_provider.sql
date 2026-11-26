-- Schema v21: how an account signs in (M3-02, #44). NULL (the default)
-- means with the password in secure storage. 'google' means with OAuth: the
-- refresh token is in secure storage and IMAP and SMTP log in with XOAUTH2.
ALTER TABLE accounts ADD COLUMN oauth_provider TEXT
    CHECK (oauth_provider IS NULL OR oauth_provider IN ('google'));
