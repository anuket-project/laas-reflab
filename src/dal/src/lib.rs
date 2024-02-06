//! Copyright (c) 2023 University of New Hampshire
//! SPDX-License-Identifier: MIT

#![allow(dead_code, unused_variables)]
#![feature(
    min_specialization,
    associated_type_defaults,
    never_type,
    generic_arg_infer,
    negative_impls,
    result_flattening,
    trait_alias,
)]

pub mod web;

use common::prelude::{tokio_postgres::types::FromSql, axum::async_trait};
use sha2::Digest;
use std::{
    any::type_name,
    backtrace::Backtrace,
    collections::{HashMap, VecDeque},
    hash::Hash,
    marker::PhantomData,
    str::FromStr,
};

use common::prelude::{config::*, itertools::Itertools, schemars::JsonSchema, *};
use serde::de::DeserializeOwned;
use tokio_postgres::{
    types::ToSql,
    Client,
    NoTls,
    Transaction,
};

use crate::web::{AnyWay, AnyWaySpecStr};

pub trait ToSqlObject = ToSql + Send + Sync + 'static;

#[derive(
    Serialize,
    Deserialize,
    Copy,
    Clone,
    Debug,
    Hash,
    derive_more::Into,
    derive_more::From,
    PartialEq,
    Eq,
)]
pub struct ID(uuid::Uuid);

pub use tokio_postgres::Row;

impl ID {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }

    pub fn nil() -> Self {
        Self(uuid::Uuid::nil())
    }
}

impl std::str::FromStr for ID {
    type Err = Box<dyn std::error::Error>;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(uuid::Uuid::try_parse(s)?))
    }
}

impl std::fmt::Display for ID {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl ToSql for ID {
    fn to_sql(
        &self,
        ty: &tokio_postgres::types::Type,
        out: &mut tokio_postgres::types::private::BytesMut,
    ) -> Result<tokio_postgres::types::IsNull, Box<dyn std::error::Error + Sync + Send>>
    where
        Self: Sized,
    {
        self.0.to_sql(ty, out)
        //self.0.id
    }

    fn accepts(ty: &tokio_postgres::types::Type) -> bool
    where Self: Sized {
        <uuid::Uuid as ToSql>::accepts(ty)
    }

    fn to_sql_checked(
        &self,
        ty: &tokio_postgres::types::Type,
        out: &mut tokio_postgres::types::private::BytesMut,
    ) -> Result<tokio_postgres::types::IsNull, Box<dyn std::error::Error + Sync + Send>> {
        self.0.to_sql_checked(ty, out)
    }
}

impl FromSql<'_> for ID {
    fn from_sql<'a>(ty: &tokio_postgres::types::Type, raw: &'a [u8]) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        match uuid::Uuid::from_sql(ty, raw) {
            Ok(u) => return Ok(ID(u)),
            Err(e) => return Err(e),
        }
    }

    fn accepts(ty: &tokio_postgres::types::Type) -> bool {
        <uuid::Uuid as FromSql>::accepts(ty)
    }
}

impl JsonSchema for ID {
    fn schema_name() -> String {
        uuid::Uuid::schema_name()
    }

    fn json_schema(gen: &mut schemars::gen::SchemaGenerator) -> schemars::schema::Schema {
        uuid::Uuid::json_schema(gen)
    }

    fn is_referenceable() -> bool {
        uuid::Uuid::is_referenceable()
    }
}

impl<T: DBTable> Serialize for FKey<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where S: serde::Serializer {
        self.id.serialize(serializer)
    }
}

impl<'de, T: DBTable> Deserialize<'de> for FKey<T> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where D: serde::Deserializer<'de> {
        let id = ID::deserialize(deserializer)?;

        Ok(Self {
            _p: PhantomData::default(),
            id,
        })
    }
}

impl<T: DBTable> std::fmt::Debug for FKey<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let tn = type_name::<T>();
        write!(f, "[Fk<{}> -> {}]", tn, self.id.0)
    }
}

pub struct FKey<T: DBTable> {
    id: ID,

    _p: PhantomData<T>,
}

impl<T: DBTable> JsonSchema for FKey<T> {
    fn schema_name() -> String {
        ID::schema_name()
    }

    fn json_schema(gen: &mut schemars::gen::SchemaGenerator) -> schemars::schema::Schema {
        ID::json_schema(gen)
    }
}

impl<T: DBTable> PartialEq for FKey<T> {
    fn eq(&self, other: &Self) -> bool {
        self.id.eq(&other.id)
    }
}

impl<T: DBTable> Eq for FKey<T> {}

impl<T: DBTable> Clone for FKey<T> {
    fn clone(&self) -> Self {
        Self { ..*self }
    }
}
impl<T: DBTable> Copy for FKey<T> {}

impl<T: DBTable> Hash for FKey<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state)
    }
}

impl<'a, T: DBTable> tokio_postgres::types::FromSql<'a> for FKey<T> {
    fn from_sql(
        ty: &tokio_postgres::types::Type,
        raw: &'a [u8],
    ) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        let id = uuid::Uuid::from_sql(ty, raw)?;

        Ok(FKey {
            id: ID::from(id),
            _p: PhantomData::default(),
        })
    }

    fn accepts(ty: &tokio_postgres::types::Type) -> bool {
        <uuid::Uuid as tokio_postgres::types::FromSql>::accepts(ty)
    }
}

impl<T: DBTable + std::fmt::Debug> ToSql for FKey<T> {
    fn to_sql(
        &self,
        ty: &tokio_postgres::types::Type,
        out: &mut tokio_postgres::types::private::BytesMut,
    ) -> Result<tokio_postgres::types::IsNull, Box<dyn std::error::Error + Sync + Send>>
    where
        Self: Sized,
    {
        self.id.0.to_sql(ty, out)
    }

    fn accepts(ty: &tokio_postgres::types::Type) -> bool
    where Self: Sized {
        <ID as ToSql>::accepts(ty)
        //<&Self as ToSql>::accepts(ty)
    }

    fn to_sql_checked(
        &self,
        ty: &tokio_postgres::types::Type,
        out: &mut tokio_postgres::types::private::BytesMut,
    ) -> Result<tokio_postgres::types::IsNull, Box<dyn std::error::Error + Sync + Send>> {
        self.id.0.to_sql_checked(ty, out)
    }
}

impl<T: DBTable> FKey<T> {
    pub async fn get(
        &self,
        transaction: &mut EasyTransaction<'_>,
    ) -> Result<ExistingRow<T>, anyhow::Error> {
        T::get(transaction, self.id).await
    }

    pub fn from_id(id: ID) -> Self {
        Self {
            id,
            _p: PhantomData::default(),
        }
    }

    pub fn into_id(&self) -> ID {
        self.id
    }

    /// Use this function when first creating a NewRow(T)
    /// for the self referential `id` field
    pub fn new_id_dangling() -> Self {
        Self::from_id(ID::new())
    }
}

#[derive(Clone, Debug, Copy, Hash)]
pub struct ExistingRow<T> {
    data: T,
    had_id: ID,
}

impl<T> std::ops::Deref for ExistingRow<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

impl<T> std::ops::DerefMut for ExistingRow<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.data
    }
}

/// Allows us to work with a row that may or may not
/// already exist in the database,
/// allowing a clean upsert operation
pub struct SchrodingerRow<T>(T);
impl<T: DBTable> SchrodingerRow<T> {
    pub async fn upsert(&self, client: &mut EasyTransaction<'_>) -> Result<FKey<T>, anyhow::Error> {
        self.0.upsert(client, Protect::new()).await
    }

    pub fn new(v: T) -> Self {
        Self(v)
    }
}

pub struct NewRow<T>(T);
impl<T: DBTable> NewRow<T> {
    pub async fn insert(&self, client: &mut EasyTransaction<'_>) -> Result<FKey<T>, anyhow::Error> {
        self.0.insert(client, Protect::new()).await
    }

    pub fn new(v: T) -> Self {
        Self(v)
    }
}

impl<T: DBTable> ExistingRow<T> {
    pub async fn update(&self, client: &mut EasyTransaction<'_>) -> Result<(), anyhow::Error> {
        assert_eq!(
            self.data.id(),
            self.had_id,
            "user tried to change the id of a model during update"
        );

        self.data.update(client, Protect::new()).await
    }

    pub async fn delete(self, client: &mut EasyTransaction<'_>) -> Result<(), anyhow::Error> {
        self.data.delete(client, Protect::new()).await
    }

    pub async fn get(client: &mut EasyTransaction<'_>, id: ID) -> Result<Self, anyhow::Error> {
        T::get(client, id).await
    }

    /// DO NOT USE THIS if you are not
    /// a) within the models crate AND
    /// b) are very intentionally only creating
    /// this from a T that already exists IN THE DATABASE
    pub fn from_existing(v: T) -> Self {
        let had_id = v.id();
        Self { data: v, had_id }
    }

    /// Unwraps an ExistingRow into its inner value
    pub fn into_inner(self) -> T {
        self.data
    }
}

pub struct Filter {
    field_name: String,
    value: Box<dyn ToSqlObject>,
    operation: FilterOperation,
}

pub enum FilterOperation {
    EQ,
    LT,
    GT,
    LTE,
    GTE,
    NE,
    LIKE,
    IN,
}

pub struct SelectBuilder<T> {
    filters: Vec<Filter>,
    _p: PhantomData<T>,
}

pub struct WhereBuilder<T> {
    select: SelectBuilder<T>,
    field_name: String,
}

pub trait Gotten<T> {
    async fn gotten(self, t: &mut EasyTransaction) -> Vec<Result<ExistingRow<T>, anyhow::Error>>;
}

impl<T: DBTable> Gotten<T> for Vec<FKey<T>> {
    async fn gotten(
        self,
        t: &mut EasyTransaction<'_>,
    ) -> Vec<Result<ExistingRow<T>, anyhow::Error>> {
        let mut collect = Vec::new();

        for v in self {
            collect.push(v.get(t).await)
        }

        collect
    }
}

impl<T: DBTable> WhereBuilder<T> {
    fn with_operation<U>(self, value: U, operation: FilterOperation) -> SelectBuilder<T>
    where U: ToSqlObject {
        let mut select = self.select;
        select.filters.push(Filter {
            field_name: self.field_name,
            value: Box::new(value),
            operation,
        });

        select
    }

    pub fn equals<U>(self, value: U) -> SelectBuilder<T>
    where U: ToSqlObject {
        self.with_operation(value, FilterOperation::EQ)
    }

    pub fn not_equals<U>(self, value: U) -> SelectBuilder<T>
    where U: ToSqlObject {
        self.with_operation(value, FilterOperation::NE)
    }

    pub fn like(self, pattern: &str) -> SelectBuilder<T> {
        self.with_operation(pattern.to_owned(), FilterOperation::LIKE)
    }

    pub fn within<U, const S: usize>(self, list: [U; S]) -> SelectBuilder<T>
    where U: ToSqlObject {
        self.with_operation(Vec::from(list), FilterOperation::IN)
    }
}

impl<T: DBTable> SelectBuilder<T> {
    pub fn new() -> Self {
        Self {
            filters: vec![],
            _p: Default::default(),
        }
    }

    pub fn where_field(self, field_name: &str) -> WhereBuilder<T> {
        WhereBuilder {
            select: self,
            field_name: field_name.to_owned(),
        }
    }

    pub async fn run(
        self,
        transaction: &mut EasyTransaction<'_>,
    ) -> Result<Vec<ExistingRow<T>>, anyhow::Error> {
        let where_clauses = if self.filters.is_empty() {
            format!("")
        } else {
            let clauses = self
                .filters
                .iter()
                .enumerate()
                .map(|(c, f)| {
                    let operator = match f.operation {
                        FilterOperation::EQ => " = ",
                        FilterOperation::NE => " != ",
                        FilterOperation::GT => " > ",
                        FilterOperation::GTE => " >= ",
                        FilterOperation::LT => " < ",
                        FilterOperation::LTE => " <= ",
                        FilterOperation::IN => " in ",
                        FilterOperation::LIKE => " like ",
                    };

                    let fname = &f.field_name;
                    //let value = &*f.value;
                    let idp = c + 1;
                    format!("({fname} {operator} ${idp})")
                })
                .join(" AND ");
            format!("WHERE {clauses}")
        };

        let tn = T::table_name();
        let q = format!("SELECT * FROM {tn} {where_clauses};");

        // I'm sorry
        let params: Vec<&(dyn ToSql + Sync)> = self
            .filters
            .iter()
            .map(|f| &*f.value as &(dyn ToSql + Sync))
            .collect_vec();

        let rows = transaction.query(&q, params.as_slice()).await.anyway()?;

        T::from_rows(rows)
    }
}

/// If you're making a SQL model, implement this directly
/// including `id`, `table_name`, `from_row`, `to_rowlike`, and `migrations`
///
/// If you just quickly want to make a model that throws its data in json
/// (and doesn't do foreign key validation, or relational verification stuff),
/// you can instead implement JsonModel
pub trait DBTable: Sized + 'static + Send + Sync {
    /// The name of the table this should be in
    fn table_name() -> &'static str;

    /// Returns the primary key for this table, as all DBTable
    /// must be PKed by an ID
    fn id(&self) -> ID;

    /// Create an instance of this table from a postgres Row object,
    /// returning Err() on (reasonable) failure.
    fn from_row(row: tokio_postgres::Row) -> Result<ExistingRow<Self>, anyhow::Error>;

    /// Should not be implemented by implementors of this trait,
    /// use the default in almost all cases!
    fn from_rows(rows: Vec<tokio_postgres::Row>) -> Result<Vec<ExistingRow<Self>>, anyhow::Error> {
        let mut vals = Vec::new();

        for row in rows {
            vals.push(Self::from_row(row)?);
        }

        Ok(vals)
    }

    /// Gives us a "rowlike" that has ToSql values by their column name as key
    fn to_rowlike(&self) -> Result<HashMap<&str, Box<dyn ToSqlObject>>, anyhow::Error>;

    /// Get a T: DBTable given an ID
    async fn get(
        client: &mut EasyTransaction<'_>,
        id: ID,
    ) -> Result<ExistingRow<Self>, anyhow::Error> {
        let tname = Self::table_name();
        let q = format!("SELECT * FROM {tname} WHERE id = $1;");
        let row = client.query_one(&q, &[&id]).await.anyway()?;

        Self::from_row(row)
    }

    /// Create a SelectBuilder based on some Self: DBTable
    fn select() -> SelectBuilder<Self> {
        SelectBuilder::new()
    }

    #[doc(hidden)]
    /// For inserting into the database, shhould usually not be implemented
    /// directly!
    ///
    /// This would be called by NewRow internally
    async fn insert(
        &self,
        client: &mut EasyTransaction<'_>,
        _t: Protect,
    ) -> Result<FKey<Self>, anyhow::Error> {
        let row = self.to_rowlike()?;

        let tname = Self::table_name();

        let mut columns = vec![];
        let mut params = vec![];
        let mut args = vec![];

        for (i, (c, v)) in row.iter().enumerate() {
            columns.push(c);
            params.push(format!("${}", i + 1));
            args.push(&**v);
        }

        let columns = columns.into_iter().join(", ");
        let params = params.into_iter().join(", ");

        let q = format!("INSERT INTO {tname} ({columns}) VALUES ({params});");

        let args = args
            .into_iter()
            .map(|d| d as &(dyn ToSql + Sync))
            .collect_vec();

        client.execute(q.as_str(), args.as_slice()).await.anyway()?;

        Ok(FKey::from_id(self.id()))
    }

    #[doc(hidden)]
    /// Called by SchrodingerRow<T>, should not be implemented by
    /// DBTable consumer directly--use the default impl!
    async fn upsert(
        &self,
        client: &mut EasyTransaction<'_>,
        _t: Protect,
    ) -> Result<FKey<Self>, anyhow::Error> {
        let row = self.to_rowlike()?;

        let tname = Self::table_name();

        let mut columns = vec![];
        let mut params = vec![];
        let mut args = vec![];

        for (i, (c, v)) in row.iter().enumerate() {
            columns.push(c);
            params.push(format!("${}", i + 1));
            args.push(&**v);
        }

        let update_cols = {
            // ignore the id column since that's where conflict arises
            // (according to our guard later)
            let r = columns
                .iter()
                .filter(|col| ***col == "id")
                .map(|col| format!("{col} = EXCLUDED.{col}"))
                .join(",\n");

            format!("{r}")
        };

        let columns = columns.into_iter().join(", ");
        let params = params.into_iter().join(", ");

        // UPSERT time
        let q = format!(
            "INSERT INTO {tname} ({columns})
                        VALUES ({params})
                        ON CONFLICT (id) DO UPDATE
                            SET {update_cols};"
        );

        tracing::debug!("Does an upsert using query: {q}");

        let args = args
            .into_iter()
            .map(|d| d as &(dyn ToSql + Sync))
            .collect_vec();

        client.execute(q.as_str(), args.as_slice()).await.anyway()?;

        Ok(FKey::from_id(self.id()))
    }

    #[doc(hidden)]
    /// Called by ExistingRow<T>, should not be implemented by
    /// DBTable consumer directly--use the default impl!
    async fn update(
        &self,
        client: &mut EasyTransaction<'_>,
        _t: Protect,
    ) -> Result<(), anyhow::Error> {
        let row = self.to_rowlike()?;

        let tname = Self::table_name();

        let mut columns = vec![];

        let mut args = vec![];

        for (_i, (k, v)) in row.iter().enumerate() {
            columns.push(k);
            args.push(&**v);
        }

        let pairs = columns
            .into_iter()
            .enumerate()
            .map(|(i, c)| {
                let v = i + 1;
                format!("{c} = ${v}")
            })
            .join(", ");

        let id = self.id();

        let last = args.len() + 1;
        let q = format!("UPDATE {tname} SET {pairs} WHERE id = ${last};");

        args.push(&id);
        let args = args
            .into_iter()
            .map(|d| d as &(dyn ToSql + Sync))
            .collect_vec();

        client.execute(q.as_str(), args.as_slice()).await.anyway()?;

        Ok(())
    }

    #[doc(hidden)]
    /// Called by ExistingRow<T>, should not be implemented by
    /// DBTable consumer directly--use the default impl!
    async fn delete(
        self,
        client: &mut EasyTransaction<'_>,
        _t: Protect,
    ) -> Result<(), anyhow::Error> {
        let tname = Self::table_name();
        let id = self.id();

        let q = format!("DELETE FROM {tname} WHERE id = $1;");

        client.execute(&q, &[&id]).await.anyway()?;

        Ok(())
    }

    /// The version of the struct-local schema that this struct/type
    /// definition uses (what migrations need to be applied before
    /// it can be used). If the provided version is *lower*
    /// than the current version of the table, a runtime error
    /// will be raised and the program will exit
    //fn version() -> usize;

    /// Provides the list of migrations (including table creation migration) that
    /// this type requires in order to migrate
    fn migrations() -> Vec<Migration>;
}

/// Prevents anyone from being able to accidentally call raw DBTable::get/update/delete
/// This should never be handed out externally to this crate
pub struct Protect {
    #[allow(dead_code)]
    guard: (), // intentionally private, prevents people from constructing this
}

impl Protect {
    /// This is intentionally not pub, so this is only constructable and passable by NewRow and ExistingRow
    /// Do not make this `pub`! Do not make any other constructor that is `pub`!
    fn new() -> Self {
        Self { guard: () }
    }
}

#[async_trait]
pub trait ComplexMigration {
    async fn run(&self, transaction: &mut EasyTransaction<'_>) -> Result<(), anyhow::Error>;
}

pub enum Apply {
    /// If this migration should be run as a
    /// single SQL query, provide this
    SQL(String),

    SQLMulti(Vec<String>),

    /// If this migration is just a marker and no
    /// operation needs to take place, put this in
    NOOP(),

    /// If the migration is more complicated,
    /// provide it as this and you can run your own
    /// SQL queries in a migration transaction here
    Operation(Box<dyn ComplexMigration + Send + Sync>),
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MigrationRecord {
    id: FKey<Self>,

    unique_name: String,

    apply_date: chrono::DateTime<chrono::Utc>,

    payload_hash: String,
}

impl DBTable for MigrationRecord {
    fn table_name() -> &'static str {
        "migration_records"
    }

    fn id(&self) -> ID {
        self.id.into_id()
    }

    fn from_row(row: tokio_postgres::Row) -> Result<ExistingRow<Self>, anyhow::Error> {
        Ok(ExistingRow::from_existing(Self {
            id: row.try_get("id")?,

            unique_name: row.try_get("unique_name")?,

            apply_date: row.try_get("apply_date")?,

            payload_hash: row.try_get("payload_hash")?,
        }))
    }

    fn to_rowlike(&self) -> Result<HashMap<&str, Box<dyn ToSql + Sync + Send>>, anyhow::Error> {
        let c: [(&str, Box<dyn tokio_postgres::types::ToSql + Sync + Send>); _] = [
            ("id", Box::new(self.id)),
            ("unique_name", Box::new(self.unique_name.clone())),
            ("apply_date", Box::new(self.apply_date.clone())),
            ("payload_hash", Box::new(self.payload_hash.clone())),
        ];

        Ok(c.into_iter().collect())
    }

    /// This table should never require any migration, if it does then we have messed up
    /// and need to think long and hard about how to do that
    ///
    /// Try, if at all possible, to operate within the bounds of the current
    /// interface for MigrationRecord and the current table schema!
    fn migrations() -> Vec<Migration> {
        unreachable!("Migration records are themselves not to be migrated automatically")
    }
}

/// A migration, with a description of what it is, a unique name,
/// the set of migrations that must be applied before it can be (in `depends_on`)
/// and an operation (in `apply`) to be performed in order to "make the migration happen"
pub struct Migration {
    pub unique_name: &'static str,
    pub description: &'static str,
    pub apply: Apply,

    pub depends_on: Vec<&'static str>,
}

impl Migration {
    /// Lets us have reasonable confidence that
    /// the action performed by a migration has
    /// not been erroniously changed "underneath" us
    ///
    /// If you want to add a field to a schema, *make a new migration!*
    /// Editing an existing one *will not work!* Migrations should only
    /// ever be applied once to any given database, they are not idempotent
    /// nor rerunnable!
    #[doc(hidden)]
    pub fn payload_hash(&self) -> String {
        let payload_str = match &self.apply {
            Apply::NOOP() => format!("_NOOP"),
            Apply::Operation(_o) => format!("_OPAQUE_OP"),
            Apply::SQL(s) => format!("SQL_{s}"),
            Apply::SQLMulti(s) => format!("SQL_{s:?}"),
        };
        let uname = self.unique_name;

        let to_hash = format!("{uname}_{payload_str}");

        let mut hasher = sha2::Sha256::new();

        hasher.update(to_hash.as_bytes());

        let f = hasher.finalize();
        let f = base16ct::lower::encode_string(&f);

        format!("Sha256({f})")
    }

    /// Whether this migration has been applied already
    pub async fn applied(&self, transaction: &mut EasyTransaction<'_>) -> bool {
        Self::applied_outer(self.unique_name, Some(self), transaction).await
    }

    pub async fn applied_outer(
        name: &str,
        _migration: Option<&Migration>,
        client: &mut EasyTransaction<'_>,
    ) -> bool {
        let res = client
            .query(
                "SELECT * FROM migration_records WHERE unique_name = $1;",
                &[&name],
            )
            .await
            .expect("Couldn't run query");

        if res.len() > 1 {
            panic!("Migration by same unique name applied multiple times");
        }

        res.len() >= 1
    }

    /// Apply this migration, returning any errors in application
    pub async fn apply(&self, transaction: &mut EasyTransaction<'_>) {
        let mut undone_precursors = Vec::new();
        for mig in self.depends_on.iter() {
            if !Migration::applied_outer(mig, None, transaction).await {
                undone_precursors.push(mig);
            }
        }

        if !undone_precursors.is_empty() {
            panic!(
                "Undone precursors for {}, the precursors/dependencies were: {:?}",
                self.unique_name, undone_precursors
            )
        }

        if !self.applied(transaction).await {
            tracing::info!("== Applying migration {}", self.unique_name);
            match &self.apply {
                Apply::SQLMulti(v) => {
                    tracing::info!("Running migration {}", self.unique_name);
                    for s in v {
                        let r = transaction.execute(s.as_str(), &[]).await;
                        if let Err(e) = r {
                            println!("Error applying migration {}", self.unique_name);
                            println!("Applying sql: {s}");
                            println!("{e:?}");
                            panic!("Couldn't apply migration");
                        }
                    }

                    NewRow::new(MigrationRecord {
                        id: FKey::new_id_dangling(),
                        unique_name: self.unique_name.to_owned(),
                        apply_date: chrono::Utc::now(),
                        payload_hash: self.payload_hash(),
                    })
                    .insert(transaction)
                    .await
                    .expect("Couldn't insert record that migration applied");
                }
                Apply::SQL(s) => {
                    tracing::info!("Running migration {}", self.unique_name);
                    let r = transaction.execute(s.as_str(), &[]).await;
                    if let Err(e) = r {
                        println!("Error applying migration {}", self.unique_name);
                        println!("Applying sql: {s}");
                        println!("{e:?}");
                        panic!("Couldn't apply migration");
                    }

                    NewRow::new(MigrationRecord {
                        id: FKey::new_id_dangling(),
                        unique_name: self.unique_name.to_owned(),
                        apply_date: chrono::Utc::now(),
                        payload_hash: self.payload_hash(),
                    })
                    .insert(transaction)
                    .await
                    .expect("Couldn't insert record that migration applied");
                }
                Apply::NOOP() => {
                    //
                }
                Apply::Operation(q) => {
                    tracing::info!("Running migration {}", self.unique_name);
                    let r = q.run(transaction).await;
                    if let Err(e) = r {
                        println!("Error applying migration {}", self.unique_name);
                        println!("{e:?}");
                        panic!("Couldn't apply migration");
                    }

                    NewRow::new(MigrationRecord {
                        id: FKey::new_id_dangling(),
                        unique_name: self.unique_name.to_owned(),
                        apply_date: chrono::Utc::now(),
                        payload_hash: self.payload_hash(),
                    })
                    .insert(transaction)
                    .await
                    .expect("Couldn't insert record that migration applied");
                },
            };
        } else {
            tracing::info!("Tried to reapply {}", self.unique_name);
        }
    }
}


pub struct ClientPair {
    client: Client,
}

impl std::ops::Deref for ClientPair {
    type Target = Client;

    fn deref(&self) -> &Self::Target {
        &self.client
    }
}

impl std::ops::DerefMut for ClientPair {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.client
    }
}

pub async fn new_client() -> Result<ClientPair, anyhow::Error> {
    let DatabaseConfig {
        url,
        username,
        password,
        database_name,
    } = settings().database.clone();

    let (client, conn) = tokio_postgres::config::Config::new()
        .user(&username)
        .password(&password)
        .dbname(&database_name)
        .host(url.host.as_str())
        .port(url.port)
        .connect(NoTls)
        .await
        .anyway()?;

    tokio::spawn(async move {
        let conn_res = conn.await;

        tracing::trace!("Result from connection after resolution: {conn_res:?}");
    });

    Ok(ClientPair { client })
}

pub trait AsEasyTransaction {
    async fn easy_transaction(&mut self) -> Result<EasyTransaction, anyhow::Error>;
}

impl AsEasyTransaction for Client {
    async fn easy_transaction(&mut self) -> Result<EasyTransaction, anyhow::Error> {
        let t = self.transaction().await;

        let as_s = match &t {
            Ok(_) => "a transaction".to_owned(),
            Err(e) => format!("Err({e:?})"),
        };

        tracing::trace!("Result from making transaction: {as_s}");
        Ok(EasyTransaction {
            inner: Some(t.anyway()?),
        })
    }
}

impl<'a> AsEasyTransaction for Transaction<'a> {
    async fn easy_transaction(&mut self) -> Result<EasyTransaction, anyhow::Error> {
        Ok(EasyTransaction {
            inner: Some(self.transaction().await.anyway()?),
        })
    }
}

impl<'a> AsEasyTransaction for EasyTransaction<'a> {
    async fn easy_transaction(&mut self) -> Result<EasyTransaction, anyhow::Error> {
        self.transaction().await
    }
}

pub struct EasyTransaction<'a> {
    inner: Option<Transaction<'a>>,
}

impl<'a> EasyTransaction<'a> {
    /// Only commit if `r` is true, otherwise roll back the transaction
    pub async fn end_with(self, r: bool) -> Result<(), anyhow::Error> {
        match r {
            true => self.commit().await,
            false => self.rollback().await,
        }
    }

    /// Take this transaction and roll it back, consuming the transaction in the process
    pub async fn rollback(mut self) -> Result<(), anyhow::Error> {
        let inner = self
            .inner
            .take()
            .ok_or(anyhow::Error::msg("no inner existed to roll back"))?;

        inner.commit().await.anyway()?;

        Ok(())
    }

    /// Commit this transaction within the context
    ///
    /// NOTE: if this has been created itself *within* another transaction,
    /// then you must commit the outer transaction as well--otherwise this
    /// one will not apply even though you "committed" it!
    pub async fn commit(mut self) -> Result<(), anyhow::Error> {
        let inner = self
            .inner
            .take()
            .ok_or(anyhow::Error::msg("no inner existed to commit"))?;

        inner.commit().await.anyway()?;

        Ok(())
    }

    /// Create a nested transaction within this transaction
    pub async fn transaction(&mut self) -> Result<EasyTransaction, anyhow::Error> {
        let inner = self
            .inner
            .as_mut()
            .ok_or("no inner to take transaction from")
            .anyway()?;
        let t = inner.transaction().await.anyway()?;

        Ok(EasyTransaction { inner: Some(t) })
    }
}

// allow calling regular Transaction methods on an EasyTransaction
impl<'a> std::ops::Deref for EasyTransaction<'a> {
    type Target = Transaction<'a>;

    fn deref(&self) -> &Self::Target {
        self.inner.as_ref().unwrap()
    }
}

// allow calling regular Transaction methods on an EasyTransaction
impl<'a> std::ops::DerefMut for EasyTransaction<'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.inner.as_mut().unwrap()
    }
}

// Transactions shouldn't be dropped in non-panicking situations,
// note that when it *does* happen it rolls back the contents of the transaction!
impl<'a> std::ops::Drop for EasyTransaction<'a> {
    fn drop(&mut self) {
        if self.inner.is_some() {
            tracing::warn!("Dropping a transaction without doing anything with it");
            let bt = Backtrace::capture();

            tracing::info!("{}", bt.to_string());

            //tracing::info!("{bt:#?}");

            tracing::warn!("End of bt");
        }
    }
}

pub struct Migrate {
    pub to_get: fn() -> Vec<Migration>,
}

impl Migrate {
    pub const fn new(callable: fn() -> Vec<Migration>) -> Self {
        Self { to_get: callable }
    }
}

inventory::collect!(Migrate);

fn all_migrations() -> std::collections::VecDeque<Migration> {
    let mut all_migrations = std::collections::VecDeque::new();

    for ms in inventory::iter::<Migrate> {
        let submigrations = ms.to_get;
        let submigrations = submigrations();

        for mig in submigrations {
            all_migrations.push_back(mig);
        }
    }

    all_migrations
}

/// This is called before any queries occur at program start,
/// and is responsible for applying migrations and verifying
/// DB integrity
pub async fn initialize() -> Result<(), Vec<common::prelude::anyhow::Error>> {
    tracing::info!("Connecting to db");
    let mut client = new_client().await.map_err(|e| vec![e])?;
    tracing::info!("Got client");
    let mut t = match client.easy_transaction().await.map_err(|e| vec![e]) {
        Ok(t) => t,
        Err(e) => {
            tracing::info!("Failed to open conn");
            return Err(e);
        }
    };
    tracing::info!("Created transaction");

    t.execute(
        "CREATE TABLE IF NOT EXISTS migration_records (
        id UUID PRIMARY KEY NOT NULL,
        unique_name VARCHAR NOT NULL,
        apply_date TIMESTAMP WITH TIME ZONE NOT NULL,
        payload_hash VARCHAR NOT NULL
    );",
        &[],
    )
    .await
    .expect("couldn't create migrations table");

    let mut errors = Vec::new();

    let g: VecDeque<Migration> = all_migrations();
    for migration in g {
        let a = migration.unique_name;
        println!("Gathered migration: {a}");
    }
    let mut all_migrations = all_migrations();

    // this is inefficient, but also only runs once on program startup
    // so I want the code for it to be simple and bulletproof
    // and not try to do anything "smart".
    // We will only likely grow to a few hundred migrations at most,
    // and even if we do it would only be a one time
    // operation to apply the initial migrations
    // most of the time, we only have maybe a few or at most 10-20
    // migrations to apply in one go to prod, so N^2 here is not
    // a meaningful perf hit

    let mut making_progress = true;

    while making_progress {
        tracing::info!("trying to apply stuff");
        making_progress = false; // we always want to have some migration available to apply

        let to_try_apply = all_migrations;
        all_migrations = std::collections::VecDeque::new(); // take the old set out

        for migration in to_try_apply {
            // check whether this migration is allowed to apply
            // based on its dependencies
            let mut can_apply = true;

            for dep in migration.depends_on.iter() {
                if !Migration::applied_outer(dep, None, &mut t).await {
                    can_apply = false;
                }
            }

            if can_apply {
                // all deps have been applied, can apply it
                migration.apply(&mut t).await;
                tracing::info!(
                    "applied {} {}",
                    migration.unique_name,
                    migration.description
                );

                making_progress = true;
            } else {
                // throw it back in to try later after other applications
                all_migrations.push_back(migration);
            }
        }
    }

    // everything that *could* be applied was, anything left is impossible to apply
    for v in all_migrations {
        let msg = format!(
            "Could not apply migration {}, dependencies were unsatisfied. The migration is: {}",
            v.unique_name, v.description
        );
        tracing::info!("{msg}");
        errors.push(anyhow::Error::msg(msg));
    }

    if errors.is_empty() {
        t.commit().await.expect("commit of migrations failed");
        Ok(())
    } else {
        t.rollback()
            .await
            .expect("couldn't rollback migrations transaction");
        Err(errors)
    }
}

pub fn col(name: &'static str, v: impl ToSqlObject) -> (&'static str, Box<dyn ToSqlObject>) {
    (name, Box::new(v))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SqlAsJson<T>(pub T)
where T: std::fmt::Debug + Clone;

impl<T> SqlAsJson<T>
where T: std::fmt::Debug + Clone
{
    pub fn extract(self) -> T {
        self.0
    }

    pub fn of(val: T) -> Self {
        Self(val)
    }
}

impl<T> ToSql for SqlAsJson<T>
where T: Serialize + DeserializeOwned + std::fmt::Debug + Clone
{
    fn to_sql(
        &self,
        ty: &tokio_postgres::types::Type,
        out: &mut tokio_postgres::types::private::BytesMut,
    ) -> Result<tokio_postgres::types::IsNull, Box<dyn std::error::Error + Sync + Send>>
    where
        Self: Sized,
    {
        serde_json::to_value(self)?.to_sql(ty, out)
    }

    fn accepts(ty: &tokio_postgres::types::Type) -> bool
    where Self: Sized {
        <serde_json::Value as ToSql>::accepts(ty)
    }

    fn to_sql_checked(
        &self,
        ty: &tokio_postgres::types::Type,
        out: &mut tokio_postgres::types::private::BytesMut,
    ) -> Result<tokio_postgres::types::IsNull, Box<dyn std::error::Error + Sync + Send>> {
        serde_json::to_value(self)?.to_sql_checked(ty, out)
    }
}

impl<'a, T> FromSql<'a> for SqlAsJson<T>
where T: Serialize + DeserializeOwned + std::fmt::Debug + Clone
{
    fn accepts(ty: &tokio_postgres::types::Type) -> bool
    where Self: Sized {
        <serde_json::Value as FromSql>::accepts(ty)
    }

    fn from_sql(
        ty: &tokio_postgres::types::Type,
        raw: &'a [u8],
    ) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        let val = serde_json::Value::from_sql(ty, raw)?;

        Ok(serde_json::from_value(val)?)
    }
}
