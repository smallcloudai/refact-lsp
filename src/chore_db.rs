use std::sync::Arc;
use tokio::sync::Mutex as AMutex;
use sea_orm::{Database, DatabaseConnection};
use crate::chore_schema::ChoreDB;
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