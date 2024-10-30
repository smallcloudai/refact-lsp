use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(ChatThreads::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(ChatThreads::CThreadId)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(ChatThreads::CThreadTitle).string().not_null())
                    .col(ColumnDef::new(ChatThreads::CThreadToolset).string().not_null())
                    .col(ColumnDef::new(ChatThreads::CThreadModelUsed).string().not_null())
                    .col(ColumnDef::new(ChatThreads::CThreadError).string().not_null())
                    .col(ColumnDef::new(ChatThreads::CThreadAnythingNew).boolean().not_null())
                    .col(ColumnDef::new(ChatThreads::CThreadCreatedTs).double().not_null())
                    .col(ColumnDef::new(ChatThreads::CThreadUpdatedTs).double().not_null())
                    .col(ColumnDef::new(ChatThreads::CThreadArchivedTs).double().not_null())
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(ChatThreads::Table).to_owned())
            .await
    }
}

#[derive(Iden)]
enum ChatThreads {
    Table,
    CThreadId,
    CThreadTitle,
    CThreadToolset,
    CThreadModelUsed,
    CThreadError,
    CThreadAnythingNew,
    CThreadCreatedTs,
    CThreadUpdatedTs,
    CThreadArchivedTs,
}