//! Typed mapping helpers for PostgreSQL rows; all SQL uses native $n parameters.
use crate::{Result, StateError};
use postgres::{types::ToSql, Row, Transaction};
pub(crate) trait QueryExt {
    fn select_one<T>(
        &mut self,
        sql: &str,
        args: &[&(dyn ToSql + Sync)],
        decode: impl FnOnce(&Row) -> std::result::Result<T, postgres::Error>,
    ) -> Result<T>;
    fn select_optional<T>(
        &mut self,
        sql: &str,
        args: &[&(dyn ToSql + Sync)],
        decode: impl FnOnce(&Row) -> std::result::Result<T, postgres::Error>,
    ) -> Result<Option<T>>;
    fn select_many<T>(
        &mut self,
        sql: &str,
        args: &[&(dyn ToSql + Sync)],
        decode: impl Fn(&Row) -> std::result::Result<T, postgres::Error>,
    ) -> Result<Vec<Result<T>>>;
}
impl QueryExt for Transaction<'_> {
    fn select_one<T>(
        &mut self,
        sql: &str,
        args: &[&(dyn ToSql + Sync)],
        decode: impl FnOnce(&Row) -> std::result::Result<T, postgres::Error>,
    ) -> Result<T> {
        Ok(decode(&self.query_one(sql, args)?)?)
    }
    fn select_optional<T>(
        &mut self,
        sql: &str,
        args: &[&(dyn ToSql + Sync)],
        decode: impl FnOnce(&Row) -> std::result::Result<T, postgres::Error>,
    ) -> Result<Option<T>> {
        self.query_opt(sql, args)?
            .map(|r| decode(&r).map_err(StateError::from))
            .transpose()
    }
    fn select_many<T>(
        &mut self,
        sql: &str,
        args: &[&(dyn ToSql + Sync)],
        decode: impl Fn(&Row) -> std::result::Result<T, postgres::Error>,
    ) -> Result<Vec<Result<T>>> {
        Ok(self
            .query(sql, args)?
            .iter()
            .map(|r| decode(r).map_err(StateError::from))
            .collect())
    }
}
macro_rules! sql_params { ($($value:expr),* $(,)?) => { &[$(&$value as &(dyn postgres::types::ToSql + Sync)),*] }; }
pub(crate) use sql_params;
