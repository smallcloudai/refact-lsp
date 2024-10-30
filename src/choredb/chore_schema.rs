use std::sync::Arc;
use serde::{Serialize, Deserialize};
use sea_orm::{
    DatabaseConnection, DeriveEntityModel, ColumnTrait, EntityTrait, Set, RelationTrait, Linked, RelationDef, EnumIter,
};

// pub mod chores {
//     use super::*;
//     use sea_orm::DerivePrimaryKey;
//     use sea_orm::PrimaryKeyTrait;
//     use sea_orm::DeriveRelation;

//     #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
//     #[sea_orm(table_name = "chores")]
//     pub struct Model {
//         #[sea_orm(primary_key)]
//         pub chore_id: String,
//         pub chore_title: String,
//         pub chore_spontaneous_work_enable: bool,
//         #[sea_orm(column_type = "Text", nullable)]
//         pub chore_event_ids: Vec<String>,
//     }

//     #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
//     pub enum Relation {
//         #[sea_orm(has_many = "crate::choredb::chore_schema::chore_events::Model")]
//         ChoreEvent,
//         #[sea_orm(has_many = "crate::choredb::chore_schema::chat_threads::Model", on_delete = "Cascade")]
//         ChatThread,
//     }

//     impl Linked for Model {
//         type FromEntity = Model;
//         type ToEntity = super::chore_events::Model;

//         fn link(&self) -> Vec<RelationDef> {
//             vec![Relation::ChoreEvent.def()]
//         }
//     }
// }

// mod chore_events {
//     use super::*;
//     use sea_orm::DerivePrimaryKey;
//     use sea_orm::PrimaryKeyTrait;
//     use sea_orm::DeriveRelation;

//     #[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
//     #[sea_orm(table_name = "chore_events")]
//     pub struct Model {
//         #[sea_orm(primary_key)]
//         pub chore_event_id: String,
//         pub chore_event_ts: f64,
//         pub chore_event_summary: String,
//         pub chore_event_link: String,
//         pub chore_event_cthread_id: String,
//     }

//     impl Linked for Model {
//         type FromEntity = Model;
//         type ToEntity = super::chat_threads::Model;

//         fn link(&self) -> Vec<RelationDef> {
//             vec![super::chores::Relation::ChatThread.def()]
//         }
//     }
// }

pub mod chat_threads {
    // use super::*;
    use sea_orm::entity::prelude::*;
    // use crate::call_validation::ChatMessage; // Adjust the path as necessary
    // use sea_orm::{DerivePrimaryKey, PrimaryKeyTrait, DeriveEntityModel, ColumnTrait, EntityTrait, Set, RelationTrait, Linked, RelationDef, EnumIter, DeriveRelation, ActiveModelBehavior};

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "chat_threads")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub cthread_id: String,
        // #[serde(default)]
        // pub cthread_messages: Vec<ChatMessage>,
        pub cthread_title: String,
        pub cthread_toolset: String,      // quick/explore/agent
        pub cthread_model_used: String,
        pub cthread_error: String,        // assign to special value "pause" to avoid auto repost to the model
        pub cthread_anything_new: bool,   // the ⚪
        pub cthread_created_ts: f64,
        pub cthread_updated_ts: f64,
        pub cthread_archived_ts: f64,     // associated container died, cannot continue
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {
        // Define relationships here if needed
    }

    impl ActiveModelBehavior for ActiveModel {

    }
}

pub struct ChoreDB {
    pub connection: Arc<DatabaseConnection>,
}

