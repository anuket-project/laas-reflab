#![doc = include_str!("../README.md")]
#![feature(
    min_specialization,
    associated_type_defaults,
    never_type,
    generic_arg_infer,
    negative_impls,
    result_flattening,
    trait_alias
)]

pub mod web;

use common::prelude::{
    anyhow::{anyhow, Error},
    tokio_postgres::types::FromSql,
};
use std::{
    any::type_name, backtrace::Backtrace, collections::HashMap, hash::Hash, marker::PhantomData,
    path::PathBuf,
};

use common::prelude::{itertools::Itertools, schemars::JsonSchema, *};
use config::settings;
use serde::de::DeserializeOwned;
use tokio_postgres::{types::ToSql, Client, NoTls, Transaction};

use crate::web::{AnyWay, AnyWaySpecStr};
use sqlx::{postgres::PgPoolOptions, PgPool};

pub trait ToSqlObject = ToSql + Send + Sync + 'static;

pub async fn get_db_pool() -> Result<PgPool, sqlx::Error> {
    let db_config = settings().database.clone();

    let connection_str = format!(
        "postgres://{}:{}@{}:{}/{}",
        db_config.username,
        db_config.password,
        db_config.url.host,
        db_config.url.port,
        db_config.database_name
    );

    PgPoolOptions::new()
        .max_connections(2)
        .connect(&connection_str)
        .await
}
pub async fn initialize() -> Result<(), Vec<Error>> {
    tracing::warn!("Setting up the database connection pool");

    let pool = get_db_pool().await.map_err(|e| vec![e.into()])?;
    tracing::warn!("Migrations running");

    if let Err(e) = sqlx::migrate!("../../migrations").run(&pool).await {
        return Err(vec![e.into()]);
    }

    Ok(())
}

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

/// UUID impl
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
    where
        Self: Sized,
    {
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
    fn from_sql<'a>(
        ty: &tokio_postgres::types::Type,
        raw: &'a [u8],
    ) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
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
    where
        S: serde::Serializer,
    {
        self.id.serialize(serializer)
    }
}

impl<'de, T: DBTable> Deserialize<'de> for FKey<T> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
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

impl<T: DBTable> Default for FKey<T> {
    fn default() -> Self {
        Self::new_id_dangling()
    }
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
    where
        Self: Sized,
    {
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

pub fn col(name: &'static str, v: impl ToSqlObject) -> (&'static str, Box<dyn ToSqlObject>) {
    (name, Box::new(v))
}

#[derive(Clone, Debug, Copy, Hash)]
pub struct ExistingRow<T: DBTable> {
    data: T,
    had_id: ID,
}

impl<T: DBTable> ExistingRow<T> {
    pub fn mass_update(&mut self, new_data: T) -> Result<(), anyhow::Error> {
        if self.data.id() == new_data.id() {
            self.data = new_data;
            Ok(())
        } else {
            Err(anyhow::Error::msg("Unable to update, IDs do not match"))
        }
    }
}

impl<T: DBTable> std::ops::Deref for ExistingRow<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

impl<T: DBTable> std::ops::DerefMut for ExistingRow<T> {
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
        let res = self.data.update(client, Protect::new()).await;
        res
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

pub trait Gotten<T: DBTable> {
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
    where
        U: ToSqlObject,
    {
        let mut select = self.select;
        select.filters.push(Filter {
            field_name: self.field_name,
            value: Box::new(value),
            operation,
        });

        select
    }

    pub fn equals<U>(self, value: U) -> SelectBuilder<T>
    where
        U: ToSqlObject,
    {
        self.with_operation(value, FilterOperation::EQ)
    }

    pub fn not_equals<U>(self, value: U) -> SelectBuilder<T>
    where
        U: ToSqlObject,
    {
        self.with_operation(value, FilterOperation::NE)
    }

    pub fn like(self, pattern: &str) -> SelectBuilder<T> {
        self.with_operation(pattern.to_owned(), FilterOperation::LIKE)
    }

    pub fn within<U, const S: usize>(self, list: [U; S]) -> SelectBuilder<T>
    where
        U: ToSqlObject,
    {
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

pub trait Named {
    fn name_parts(&self) -> Vec<String>;
    fn name_columnnames() -> Vec<String>;
}

pub trait Lookup: DBTable + Named {
    async fn lookup(
        transaction: &mut EasyTransaction<'_>,
        name_parts: Vec<String>,
    ) -> Result<ExistingRow<Self>, anyhow::Error> {
        let mut select = Self::select();
        let col_names = Self::name_columnnames();

        for (col, val) in col_names.into_iter().zip(name_parts.into_iter()) {
            select = select.where_field(&col).equals(val);
        }

        let mut res = select
            .run(transaction)
            .await
            .expect("Expected to run query");

        match res.len() {
            0 => Err(anyhow!("No results found")),
            1 => Ok(res.pop().unwrap()),
            _ => panic!("Someone did not implement Named properly."),
        }
    }
}

pub trait Importable: Lookup {
    async fn import(
        transaction: &mut EasyTransaction<'_>,
        import_file_path: PathBuf,
        proj_path: Option<PathBuf>,
    ) -> Result<Option<ExistingRow<Self>>, anyhow::Error>;
    async fn export(&self, transaction: &mut EasyTransaction<'_>) -> Result<(), anyhow::Error>;
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

    // For inserting into the database, should usually not be implemented
    // directly!
    //
    // This would be called by NewRow internally
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

    // Called by SchrodingerRow<T>, should not be implemented by
    // DBTable consumer directly--use the default impl!
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

    // Called by ExistingRow<T>, should not be implemented by
    // DBTable consumer directly--use the default impl!
    async fn update(
        &self,
        client: &mut EasyTransaction<'_>,
        _t: Protect,
    ) -> Result<(), anyhow::Error> {
        let row = self.to_rowlike()?;

        let tname = Self::table_name();

        let mut columns = vec![];

        let mut args = vec![];
        for (k, v) in row.iter() {
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

    // Called by ExistingRow<T>, should not be implemented by
    // DBTable consumer directly--use the default impl!
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
    let config::DatabaseConfig {
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
    /// Take this transaction and roll it back, consuming the transaction in the process
    pub async fn rollback(mut self) -> Result<(), anyhow::Error> {
        let inner = self
            .inner
            .take()
            .ok_or(anyhow::Error::msg("no inner existed to roll back"))?;

        inner.rollback().await.anyway()?;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SqlAsJson<T>(pub T)
where
    T: std::fmt::Debug + Clone;

impl<T> SqlAsJson<T>
where
    T: std::fmt::Debug + Clone,
{
    pub fn extract(self) -> T {
        self.0
    }

    pub fn of(val: T) -> Self {
        Self(val)
    }
}

impl<T> ToSql for SqlAsJson<T>
where
    T: Serialize + DeserializeOwned + std::fmt::Debug + Clone,
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
    where
        Self: Sized,
    {
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
where
    T: Serialize + DeserializeOwned + std::fmt::Debug + Clone,
{
    fn accepts(ty: &tokio_postgres::types::Type) -> bool
    where
        Self: Sized,
    {
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
