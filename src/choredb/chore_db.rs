use std::sync::Arc;
use tokio::sync::Mutex as AMutex;
use sea_orm::{Database, DatabaseConnection};
use crate::choredb::chore_schema::ChoreDB;
// use sea_orm::{Database, DatabaseConnection, EntityTrait};
// use crate::chore_schema::{ChoreDB, Chore, ChoreEvent, ChatThread};
// use crate::call_validation::ChatMessage;


pub async fn chore_db_init(database_path: String) -> Arc<AMutex<ChoreDB>> {
    let database_url = format!("sqlite://{}", database_path);
    let connection = Database::connect(&database_url).await.unwrap();
    Arc::new(AMutex::new(ChoreDB {
        connection: Arc::new(connection),
    }))
}

// // Getters

// pub async fn chore_get(
//     cdb: Arc<AMutex<ChoreDB>>,
//     chore_id: String,
// ) -> Option<Chore> {
//     let db = cdb.lock().await.connection.clone();
//     Chore::find_by_id(chore_id).one(&*db).await.unwrap()
// }

// pub async fn chore_event_get(
//     cdb: Arc<AMutex<ChoreDB>>,
//     chore_event_id: String,
// ) -> Option<ChoreEvent> {
//     let db = cdb.lock().await.connection.clone();
//     ChoreEvent::find_by_id(chore_event_id).one(&*db).await.unwrap()
// }

// pub async fn chat_message_get(
//     cdb: Arc<AMutex<ChoreDB>>,
//     cthread_id: String,
//     i: usize,
// ) -> Option<ChatMessage> {
//     // Implement this based on your ChatMessage schema and SeaORM setup
//     unimplemented!()
// }

// pub async fn chat_thread_get(
//     cdb: Arc<AMutex<ChoreDB>>,
//     cthread_id: String,
// ) -> Option<ChatThread> {
//     let db = cdb.lock().await.connection.clone();
//     ChatThread::find_by_id(cthread_id).one(&*db).await.unwrap()
// }

// pub async fn chat_messages_load(
//     cdb: Arc<AMutex<ChoreDB>>,
//     cthread: &mut ChatThread
// ) {
//     // Implement this based on your ChatMessage schema and SeaORM setup
//     unimplemented!()
// }

// // Setters

// pub async fn chore_set(
//     cdb: Arc<AMutex<ChoreDB>>,
//     chore: Chore,
// ) {
//     let db = cdb.lock().await.connection.clone();
//     Chore::insert(chore).exec(&*db).await.unwrap();
// }

// pub async fn chore_event_set(
//     cdb: Arc<AMutex<ChoreDB>>,
//     chore_event: ChoreEvent,
// ) {
//     let db = cdb.lock().await.connection.clone();
//     ChoreEvent::insert(chore_event).exec(&*db).await.unwrap();
// }

// pub async fn chat_message_set(
//     cdb: Arc<AMutex<ChoreDB>>,
//     cthread_id: String,
//     i: usize,
//     message: ChatMessage,
// ) {
//     // Implement this based on your ChatMessage schema and SeaORM setup
//     unimplemented!()
// }

// pub async fn chat_thread_set(
//     cdb: Arc<AMutex<ChoreDB>>,
//     chat_thread: ChatThread,
// ) {
//     let db = cdb.lock().await.connection.clone();
//     ChatThread::insert(chat_thread).exec(&*db).await.unwrap();
// }

// pub async fn chat_messages_save(
//     cdb: Arc<AMutex<ChoreDB>>,
//     cthread: &ChatThread,
// ) {
//     // Implement this based on your ChatMessage schema and SeaORM setup
//     unimplemented!()
// }



#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{Database, EntityTrait, ActiveModelTrait, Set, Statement};
    use sea_orm_migration::prelude::*;
    use crate::choredb::chore_schema::chat_threads;

    async fn sqlite_print_db_structure(db: &DatabaseConnection) {
        let stmt = Statement::from_string(db.get_database_backend(), "SELECT name FROM sqlite_master WHERE type='table';".to_owned());
        let tables: Vec<(String,)> = db.query_all(stmt)
            .await.unwrap()
            .into_iter()
            .map(|row| (row.try_get::<String>("", "name").unwrap(),))
            .collect();

        for (table,) in tables {
            println!("Table: {}", table);
            let stmt = Statement::from_string(db.get_database_backend(), format!("PRAGMA table_info({});", table));
            let columns: Vec<(String, String)> = db.query_all(stmt)
                .await.unwrap()
                .into_iter()
                .map(|row| (row.try_get("", "name").unwrap(), row.try_get("", "type").unwrap()))
                .collect();

            for (name, type_) in columns {
                println!("  Column: {} {}", name, type_);
            }
        }
    }

    async fn sqlite_print_table_records(db: &DatabaseConnection, table_name: String, limit: usize) {
        println!("Table: {}", table_name);
        let stmt = Statement::from_string(db.get_database_backend(), format!("SELECT * FROM {} LIMIT {};", table_name, limit));
        let rows = db.query_all(stmt).await.unwrap();

        for row in rows {
            let columns = row.column_names();
            for column in columns {
                // let value: String = row.try_get("", column.as_str()).unwrap_or("NULL".to_string());
                println!("  {}: {:?}", column, row.try_get::<String>("", column.as_str()));
            }
            println!("--------");
        }
    }

    #[tokio::test]
    async fn test_chat_threads_crud() {
        let db_url = "sqlite::memory:";
        let db = Database::connect(db_url).await.unwrap();
        let schema_manager = SchemaManager::new(&db);
        let migration = crate::choredb::chore_schema_m20241030::Migration;
        migration.up(&schema_manager).await.unwrap();
        sqlite_print_db_structure(&db).await;

        let chore_db = ChoreDB {
            connection: Arc::new(db),
        };

        let chat_thread1 = chat_threads::ActiveModel {
            cthread_id: Set("thread1".to_owned()),
            cthread_title: Set("First Thread".to_owned()),
            cthread_toolset: Set("quick".to_owned()),
            cthread_model_used: Set("model1".to_owned()),
            cthread_error: Set("".to_owned()),
            cthread_anything_new: Set(false),
            cthread_created_ts: Set(1627847267.0),
            cthread_updated_ts: Set(1627847267.0),
            cthread_archived_ts: Set(0.0),
        };

        let chat_thread2 = chat_threads::ActiveModel {
            cthread_id: Set("thread2".to_owned()),
            cthread_title: Set("Second Thread".to_owned()),
            cthread_toolset: Set("explore".to_owned()),
            cthread_model_used: Set("model2".to_owned()),
            cthread_error: Set("".to_owned()),
            cthread_anything_new: Set(true),
            cthread_created_ts: Set(1627847268.0),
            cthread_updated_ts: Set(1627847268.0),
            cthread_archived_ts: Set(0.0),
        };

        let aa = chat_thread1.insert(&*chore_db.connection).await;
        sqlite_print_table_records(&*chore_db.connection, "chat_threads".to_string(), 2).await;
        let threads: Vec<chat_threads::Model> = chat_threads::Entity::find().all(&*chore_db.connection).await.unwrap();
        if threads.is_empty() {
            println!("No chat threads found");
        } else {
            for thread in threads {
                println!("Retrieved thread: {:?}", thread);
            }
        }
        aa.unwrap();
        chat_thread2.insert(&*chore_db.connection).await.unwrap();

        let threads: Vec<chat_threads::Model> = chat_threads::Entity::find().all(&*chore_db.connection).await.unwrap();
        for thread in threads {
            println!("{:?}", thread);
        }
    }
}
