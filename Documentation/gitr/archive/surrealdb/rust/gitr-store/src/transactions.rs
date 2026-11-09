//! Inspect every statement: the first failure may only be NotExecuted because
//! a later statement caused the transaction to abort. Never retry unknown errors.
use surrealdb::{Error, IndexedResults, types::QueryError};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Reason { Retry, Skipped, Fatal }

fn reason(error: &Error) -> Reason {
    let mut current = Some(error);
    let mut skipped = false;
    for _ in 0..32 {
        let Some(error) = current else {
            return if skipped { Reason::Skipped } else { Reason::Fatal };
        };
        if matches!(error.query_details(), Some(QueryError::TransactionConflict))
            || error.is_already_exists()
        {
            return Reason::Retry;
        }
        // This marker is authored by this service, not inferred from a database
        // failure's prose. Both representations are qualified against pinned 4.0.
        if error.is_thrown() && matches!(error.message(), "gitr_cas" | "An error occurred: gitr_cas") {
            return Reason::Retry;
        }
        if matches!(error.query_details(), Some(QueryError::NotExecuted)) {
            skipped = true;
        } else if !error.is_context() {
            return Reason::Fatal;
        }
        current = error.cause();
    }
    Reason::Fatal
}

pub(super) fn retryable(error: &Error) -> bool { reason(error) == Reason::Retry }

pub(super) fn check(mut response: IndexedResults) -> surrealdb::Result<IndexedResults> {
    let mut conflict = None;
    let mut skipped = None;
    for (_, error) in response.take_errors() {
        match reason(&error) {
            Reason::Fatal => return Err(error),
            Reason::Retry => { if conflict.is_none() { conflict = Some(error); } }
            Reason::Skipped => { if skipped.is_none() { skipped = Some(error); } }
        }
    }
    match conflict.or(skipped) {
        Some(error) => Err(error),
        None => Ok(response),
    }
}
