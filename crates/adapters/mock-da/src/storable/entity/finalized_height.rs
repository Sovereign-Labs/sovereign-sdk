use sea_orm::entity::prelude::*;
use sea_orm::sea_query::OnConflict;
use sea_orm::ActiveValue::Set;

/// Single row table for recording last finalized height.
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "last_finalized_height")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub value: i32,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

pub const ID: i32 = 1;

/// "upsert" the last finalized height
pub async fn update_value(
    db: &DatabaseConnection,
    last_finalized_height: u32,
) -> Result<(), DbErr> {
    let insert_stmt = Entity::insert(ActiveModel {
        id: Set(ID),
        value: Set(last_finalized_height as i32),
    })
    .on_conflict(
        OnConflict::column(Column::Id)
            .update_column(Column::Value)
            .to_owned(),
    );

    insert_stmt.exec(db).await?;
    Ok(())
}
