//! Reads of the seeded profile, through `Store::conn()`'s sanctioned raw-SQL
//! escape hatch. The demo never writes through these — `seed` owns every
//! INSERT — and never reads `*_local` detail fields except the goal titles
//! the research draft explicitly drafts for the semantic scan.

use rusqlite::{Connection, OptionalExtension};

use crate::demo::StoredGoal;
use crate::error::CliError;

/// The seeded household's row: id plus the generalized facts the store
/// holds (region class, income-band label via the `band` join).
#[derive(Debug)]
pub struct Household {
    pub id: String,
    pub region_class: String,
    pub income_band: Option<String>,
}

/// Load the demo household. `QueryReturnedNoRow` is surfaced as a
/// [`CliError::Demo`] telling the user what to do, not a bare sqlite error.
pub fn load_household(conn: &Connection) -> Result<Household, CliError> {
    conn.query_row(
        "SELECT h.id, h.region_class, b.label
         FROM household h
         LEFT JOIN band b ON b.id = h.income_band_id",
        [],
        |row| {
            Ok(Household {
                id: row.get(0)?,
                region_class: row.get(1)?,
                income_band: row.get(2)?,
            })
        },
    )
    .optional()
    .map_err(CliError::from)?
    .ok_or_else(|| {
        CliError::Demo(
            "no seeded family in this database — run `ha-cli seed --db <path>` first".to_string(),
        )
    })
}

/// Load the seeded goals in one domain, highest importance first — the
/// purpose-limited slice the research draft draws on.
pub fn load_goals(
    conn: &Connection,
    household_id: &str,
    domain: &str,
) -> Result<Vec<StoredGoal>, CliError> {
    let mut statement = conn
        .prepare(
            "SELECT title_local, domain, importance
             FROM goal
             WHERE household_id = ?1 AND domain = ?2
             ORDER BY importance DESC, id",
        )
        .map_err(CliError::from)?;
    let rows = statement
        .query_map([household_id, domain], |row| {
            Ok(StoredGoal {
                title_local: row.get(0)?,
                domain: row.get(1)?,
                importance: row.get(2)?,
            })
        })
        .map_err(CliError::from)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(CliError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo;
    use ha_store::StoreKey;

    fn open_store() -> ha_store::Store {
        let key = StoreKey::from_passphrase("test-key").unwrap();
        ha_store::Store::open_in_memory(&key).unwrap()
    }

    #[test]
    fn an_empty_store_reads_as_no_seeded_family() {
        let mut store = open_store();
        let error = load_household(store.conn()).unwrap_err();
        assert!(error.to_string().contains("no seeded family"));
    }

    #[test]
    fn goals_load_in_importance_order_for_one_domain() {
        let mut store = open_store();
        let conn = store.conn();
        seed_minimal(conn);

        let goals = load_goals(conn, demo::HOUSEHOLD_ID, "wealth").unwrap();
        assert_eq!(goals.len(), 1);
        assert_eq!(goals[0].title_local, "Build a six-month emergency fund");

        // Other domains are reachable too; the demo only drafts wealth.
        assert_eq!(
            load_goals(conn, demo::HOUSEHOLD_ID, "health")
                .unwrap()
                .len(),
            1
        );
        assert!(load_goals(conn, demo::HOUSEHOLD_ID, "career")
            .unwrap()
            .is_empty());
    }

    /// The few rows `load_goals` needs; the full family seed lives in
    /// `commands::seed`.
    fn seed_minimal(conn: &Connection) {
        // The band taxonomy ships with the migrations; the fixture only
        // adds the family rows that reference it.
        conn.execute_batch(
            "INSERT INTO household (id, timezone, region_class) VALUES
                 ('household_demo', 'America/Chicago', 'urban_metro');
             INSERT INTO person (id, household_id, role, display_name_local, age_band_id, is_child)
                 VALUES ('person_sam', 'household_demo', 'adult', 'Sam', 'age_35_44', 0);
             INSERT INTO goal (id, household_id, person_id, title_local, domain, importance)
                 VALUES ('goal_emergency_fund', 'household_demo', NULL,
                         'Build a six-month emergency fund', 'wealth', 9);
             INSERT INTO goal (id, household_id, person_id, title_local, domain, importance)
                 VALUES ('goal_swimming', 'household_demo', 'person_sam',
                         'Learn to swim confidently', 'health', 7);",
        )
        .unwrap();
    }
}
