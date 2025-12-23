use crate::error::ApiError;
use backup_sync_protocol::SyncFolder;
use sqlx::{Pool, Sqlite};
use uuid::Uuid;

pub async fn create_folder(
    db: &Pool<Sqlite>,
    user_id: &str,
    name: &str,
    computer_id: &str,
) -> Result<SyncFolder, ApiError> {
    computer_belongs_to_user(db, computer_id, user_id).await?;

    let folder_id = Uuid::new_v4().to_string();

    sqlx::query!(
        "INSERT INTO folders (id, name, origin_computer_id) VALUES (?, ?, ?)",
        folder_id,
        name,
        computer_id
    )
    .execute(db)
    .await?;

    Ok(SyncFolder {
        id: folder_id,
        name: name.to_string(),
        origin_computer: computer_id.to_string(),
        backup_computers: vec![],
        is_synced: false,
        pending_operations: 0,
    })
}

pub async fn join_folder(
    db: &Pool<Sqlite>,
    user_id: &str,
    folder_id: &str,
    computer_id: &str,
) -> Result<String, ApiError> {
    computer_belongs_to_user(db, computer_id, user_id).await?;

    // Verify folder exists and get owner
    let folder_owner = sqlx::query_scalar!(
        "
        SELECT c.user_id
        FROM folders f
        JOIN computers c ON f.origin_computer_id = c.id
        WHERE f.id = ?
    ",
        folder_id
    )
    .fetch_optional(db)
    .await?;

    if folder_owner != Some(user_id.to_string()) {
        return Err(ApiError::NotFound(
            "Folder not found or access denied".to_string(),
        ));
    }

    let result = sqlx::query!(
        "INSERT INTO folder_backups (folder_id, computer_id) VALUES (?, ?)",
        folder_id,
        computer_id
    )
    .execute(db)
    .await;

    match result {
        Ok(_) => Ok("Joined folder".to_owned()),
        Err(e) => {
            // Check for duplicate (already joined)
            if e.as_database_error()
                .is_some_and(sqlx::error::DatabaseError::is_unique_violation)
            {
                Ok("Already joined".to_owned())
            } else {
                Err(ApiError::DatabaseError(e))
            }
        }
    }
}

pub async fn leave_folder(
    db: &Pool<Sqlite>,
    user_id: &str,
    folder_id: &str,
    computer_id: &str,
) -> Result<(), ApiError> {
    computer_belongs_to_user(db, computer_id, user_id).await?;

    sqlx::query!(
        "DELETE FROM folder_backups WHERE folder_id = ? AND computer_id = ?",
        folder_id,
        computer_id
    )
    .execute(db)
    .await?;

    Ok(())
}

pub async fn get_folders_by_user(
    db: &Pool<Sqlite>,
    user_id: &str,
) -> Result<Vec<SyncFolder>, ApiError> {
    // Simplification: fetch folders where origin belongs to user
    let folders_data = sqlx::query!(
        "
        SELECT f.id, f.name, f.origin_computer_id, f.is_synced, f.pending_operations 
        FROM folders f
        JOIN computers c ON f.origin_computer_id = c.id
        WHERE c.user_id = ?
        GROUP BY f.id
    ",
        user_id
    )
    .fetch_all(db)
    .await?;

    let mut sync_folders = Vec::new();
    for rec in folders_data {
        // Fetch backups for this folder
        let backups_data = sqlx::query_scalar!(
            "SELECT computer_id FROM folder_backups WHERE folder_id = ?",
            rec.id
        )
        .fetch_all(db)
        .await?;

        sync_folders.push(SyncFolder {
            id: rec.id,
            name: rec.name,
            origin_computer: rec.origin_computer_id,
            backup_computers: backups_data,
            is_synced: rec.is_synced,
            pending_operations: rec.pending_operations as u64,
        });
    }

    Ok(sync_folders)
}

pub async fn get_folders_by_computer(
    db: &Pool<Sqlite>,
    user_id: &str,
    computer_id: &str,
) -> Result<Vec<SyncFolder>, ApiError> {
    computer_belongs_to_user(db, computer_id, user_id).await?;

    // Fetch folders where this computer is the origin OR where it is a backup
    let folders_data = sqlx::query!(
        "
        SELECT DISTINCT f.id, f.name, f.origin_computer_id, f.is_synced, f.pending_operations 
        FROM folders f
        LEFT JOIN folder_backups fb ON f.id = fb.folder_id
        WHERE f.origin_computer_id = ? OR fb.computer_id = ?
    ",
        computer_id,
        computer_id,
    )
    .fetch_all(db)
    .await?;

    let mut sync_folders = Vec::new();
    for rec in folders_data {
        // Fetch backups for this folder
        let backups_data = sqlx::query_scalar!(
            "SELECT computer_id FROM folder_backups WHERE folder_id = ?",
            rec.id
        )
        .fetch_all(db)
        .await?;

        sync_folders.push(SyncFolder {
            id: rec.id,
            name: rec.name,
            origin_computer: rec.origin_computer_id,
            backup_computers: backups_data,
            is_synced: rec.is_synced,
            pending_operations: rec.pending_operations as u64,
        });
    }

    Ok(sync_folders)
}

async fn computer_belongs_to_user(
    db: &Pool<Sqlite>,
    id: &str,
    user_id: &str,
) -> Result<(), ApiError> {
    // Verify computer belongs to user
    sqlx::query!(
        "SELECT id FROM computers WHERE id = ? AND user_id = ?",
        id,
        user_id
    )
    .fetch_optional(db)
    .await?
    .ok_or(ApiError::PermissionDenied(
        "Computer does not belong to user".to_owned(),
    ))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init_db;
    use crate::logic::computer::register_computer;

    #[tokio::test]
    async fn test_create_and_join_folder() {
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

        let comp1 = register_computer(&db, &user_id, "PC1").await.unwrap();
        let comp2 = register_computer(&db, &user_id, "PC2").await.unwrap();

        let folder = create_folder(&db, &user_id, "Docs", &comp1.id)
            .await
            .unwrap();
        assert_eq!(folder.name, "Docs");
        assert_eq!(folder.origin_computer, comp1.id);

        let result = join_folder(&db, &user_id, &folder.id, &comp2.id)
            .await
            .unwrap();
        assert_eq!(result, "Joined folder");

        let folders = get_folders_by_user(&db, &user_id).await.unwrap();
        assert_eq!(folders.len(), 1);
        assert_eq!(folders[0].backup_computers.len(), 1);
        assert_eq!(folders[0].backup_computers[0], comp2.id);

        // Test get_folders_by_computer
        let comp1_folders = get_folders_by_computer(&db, &user_id, &comp1.id)
            .await
            .unwrap();
        assert_eq!(comp1_folders.len(), 1); // Origin

        let comp2_folders = get_folders_by_computer(&db, &user_id, &comp2.id)
            .await
            .unwrap();
        assert_eq!(comp2_folders.len(), 1); // Backup
    }
}
