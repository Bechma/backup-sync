-- Add up migration script here
CREATE TABLE users
(
    id            TEXT PRIMARY KEY NOT NULL,
    name          TEXT             NOT NULL,
    password_hash TEXT             NOT NULL
);

CREATE TABLE computers
(
    id        TEXT PRIMARY KEY NOT NULL,
    user_id   TEXT             NOT NULL,
    name      TEXT             NOT NULL,
    online    BOOLEAN          NOT NULL DEFAULT FALSE,
    last_seen TIMESTAMP,
    FOREIGN KEY (user_id) REFERENCES users (id) ON DELETE CASCADE
);

CREATE TABLE folders
(
    id                 TEXT PRIMARY KEY NOT NULL,
    name               TEXT             NOT NULL,
    origin_computer_id TEXT             NOT NULL,
    is_synced          BOOLEAN          NOT NULL DEFAULT FALSE,
    pending_operations INTEGER          NOT NULL DEFAULT 0,
    FOREIGN KEY (origin_computer_id) REFERENCES computers (id) ON DELETE CASCADE
);

CREATE TABLE folder_backups
(
    folder_id   TEXT NOT NULL,
    computer_id TEXT NOT NULL,
    PRIMARY KEY (folder_id, computer_id),
    FOREIGN KEY (folder_id) REFERENCES folders (id) ON DELETE CASCADE,
    FOREIGN KEY (computer_id) REFERENCES computers (id) ON DELETE CASCADE
);