use crate::error::ApiError;
use backup_sync_protocol::Computer;
use sqlx::{Pool, Sqlite};
use uuid::Uuid;

pub async fn register_computer(
    db: &Pool<Sqlite>,
    user_id: &str,
    name: &str,
) -> Result<Computer, ApiError> {
    let computer_id = Uuid::new_v4().to_string();

    sqlx::query!(
        "INSERT INTO computers (id, user_id, name, online) VALUES (?, ?, ?, ?)",
        computer_id,
        user_id,
        name,
        true
    )
    .execute(db)
    .await?;

    Ok(Computer {
        id: computer_id,
        name: name.to_string(),
        online: true,
    })
}

pub async fn get_computers_by_user(
    db: &Pool<Sqlite>,
    user_id: &str,
) -> Result<Vec<Computer>, ApiError> {
    let computers = sqlx::query_as!(
        Computer,
        "SELECT id, name, online FROM computers WHERE user_id = ?",
        user_id
    )
    .fetch_all(db)
    .await?;

    Ok(computers)
}

pub async fn remove_computer(
    db: &Pool<Sqlite>,
    user_id: &str,
    computer_id: &str,
) -> Result<(), ApiError> {
    // Verify computer belongs to user
    sqlx::query!(
        "SELECT id FROM computers WHERE id = ? AND user_id = ?",
        computer_id,
        user_id
    )
    .fetch_optional(db)
    .await?
    .ok_or(ApiError::PermissionDenied(
        "Computer does not belong to user".to_owned(),
    ))?;

    // With ON DELETE CASCADE, removing the computer removes associated folders and backups
    sqlx::query!("DELETE FROM computers WHERE id = ?", computer_id)
        .execute(db)
        .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init_db;

    #[tokio::test]
    async fn test_register_and_get_computer() {
        let db = init_db().await.unwrap();
        // create user first (needed for FK)
        let user_id = Uuid::new_v4().to_string();
        sqlx::query!(
            "INSERT INTO users (id, name, password_hash) VALUES (?, ?, ?)",
            user_id,
            "testuser",
            "hash"
        )
        .execute(&db)
        .await
        .unwrap();

        let computer = register_computer(&db, &user_id, "MyPC").await.unwrap();
        assert_eq!(computer.name, "MyPC");
        assert!(computer.online);

        let computers = get_computers_by_user(&db, &user_id).await.unwrap();
        assert_eq!(computers.len(), 1);
        assert_eq!(computers[0].id, computer.id);
    }

    #[tokio::test]
    async fn test_remove_computer() {
        let db = init_db().await.unwrap();
        let user_id = Uuid::new_v4().to_string();
        sqlx::query!(
            "INSERT INTO users (id, name, password_hash) VALUES (?, ?, ?)",
            user_id,
            "testuser",
            "hash"
        )
        .execute(&db)
        .await
        .unwrap();

        let computer = register_computer(&db, &user_id, "MyPC").await.unwrap();

        remove_computer(&db, &user_id, &computer.id).await.unwrap();

        let computers = get_computers_by_user(&db, &user_id).await.unwrap();
        assert_eq!(computers.len(), 0);
    }
}
