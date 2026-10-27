use super::AppError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(default, deny_unknown_fields)]
pub struct Pagination {
    pub page: u32,
    pub per_page: u32,
}
impl Default for Pagination {
    fn default() -> Self {
        Self {
            page: 1,
            per_page: 20,
        }
    }
}
impl Pagination {
    pub fn validate(&self) -> Result<(), AppError> {
        if self.page == 0 || self.page > 100_000 || !(1..=100).contains(&self.per_page) {
            return Err(AppError::Validation(
                "page must be 1..100000 and per_page must be 1..100".into(),
            ));
        }
        Ok(())
    }
    pub fn offset(&self) -> i64 {
        i64::from(self.page.saturating_sub(1)) * i64::from(self.per_page)
    }
    pub fn limit(&self) -> i64 {
        i64::from(self.per_page)
    }
}
#[derive(Debug, Serialize)]
pub struct Paginated<T> {
    pub items: Vec<T>,
    pub page: u32,
    pub per_page: u32,
    pub total: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_unbounded_or_zero_pages() {
        for p in [
            Pagination {
                page: 0,
                per_page: 20,
            },
            Pagination {
                page: 1,
                per_page: 101,
            },
            Pagination {
                page: u32::MAX,
                per_page: 100,
            },
        ] {
            assert!(p.validate().is_err());
        }
        assert_eq!(
            Pagination {
                page: 3,
                per_page: 25
            }
            .offset(),
            50
        );
    }
}
