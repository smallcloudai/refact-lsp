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
                        ColumnDef::new(ChatThreads::CthreadId)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(ChatThreads::CthreadTitle).string().not_null())
                    .col(ColumnDef::new(ChatThreads::CthreadToolset).string().not_null())
                    .col(ColumnDef::new(ChatThreads::CthreadModelUsed).string().not_null())
                    .col(ColumnDef::new(ChatThreads::CthreadError).string().not_null())
                    .col(ColumnDef::new(ChatThreads::CthreadAnythingNew).boolean().not_null())
                    .col(ColumnDef::new(ChatThreads::CthreadCreatedTs).double().not_null())
                    .col(ColumnDef::new(ChatThreads::CthreadUpdatedTs).double().not_null())
                    .col(ColumnDef::new(ChatThreads::CthreadArchivedTs).double().not_null())
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
    CthreadId,
    CthreadTitle,
    CthreadToolset,
    CthreadModelUsed,
    CthreadError,
    CthreadAnythingNew,
    CthreadCreatedTs,
    CthreadUpdatedTs,
    CthreadArchivedTs,
}
