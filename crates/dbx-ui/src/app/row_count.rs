//! How many rows a data tab's table holds: free when the last page is in
//! view, a catalog estimate for unfiltered tables, and an exact `COUNT(*)`
//! on request.

use super::*;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum RowCount {
    #[default]
    Unknown,
    Estimate(u64),
    Counting,
    Exact(u64),
}

impl DbxApp {
    /// Update the count after a page loads with `filters` applied. A page
    /// without a successor ends the table, so its count is exact.
    pub(super) fn page_loaded_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        filters: Vec<Filter>,
        cx: &mut Context<Self>,
    ) {
        let Some(data) = self.data_tab_mut(session_id, tab_id) else {
            return;
        };
        let rows = data.result.as_ref().map_or(0, |result| result.rows.len()) as u64;
        let before = data.table_page * u64::from(TABLE_BROWSE_PAGE_SIZE);
        let same_filters = data.counted_filters == filters;
        data.counted_filters = filters;
        if !data.table_has_next_page {
            data.row_count = RowCount::Exact(before + rows);
            return;
        }
        if data.table_page > 0 && same_filters {
            return;
        }
        // Reloading page one may follow a commit, so an old figure is stale.
        data.row_count = match data.row_count {
            RowCount::Estimate(estimate) if same_filters => RowCount::Estimate(estimate),
            _ => RowCount::Unknown,
        };
        if data.counted_filters.is_empty() {
            self.estimate_rows_for(session_id, tab_id, cx);
        }
    }

    fn estimate_rows_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        cx: &mut Context<Self>,
    ) {
        let Some((engine, table)) = self.session(session_id).and_then(|session| {
            Some((
                session.engine.clone()?,
                session.data_tab(tab_id)?.table.clone(),
            ))
        }) else {
            return;
        };
        let task = self
            .runtime
            .spawn(async move { engine.estimate_rows(&table).await });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(estimate))) = task.await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                let Some(data) = this.data_tab_mut(session_id, tab_id) else {
                    return;
                };
                let loaded = data.table_page * u64::from(TABLE_BROWSE_PAGE_SIZE)
                    + data.result.as_ref().map_or(0, |result| result.rows.len()) as u64;
                // Stale statistics can undercount; never show fewer rows than
                // the pages already seen prove exist.
                if data.row_count == RowCount::Unknown
                    || matches!(data.row_count, RowCount::Estimate(_))
                {
                    data.row_count = RowCount::Estimate(estimate.max(loaded + 1));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Run an exact `COUNT(*)` with the tab's applied filters.
    pub(super) fn count_rows_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        cx: &mut Context<Self>,
    ) {
        let Some((engine, table, filters, columns)) =
            self.session(session_id).and_then(|session| {
                let data = session.data_tab(tab_id)?;
                Some((
                    session.engine.clone()?,
                    data.table.clone(),
                    data.counted_filters.clone(),
                    data.table_columns.clone(),
                ))
            })
        else {
            return;
        };
        let Some(data) = self.data_tab_mut(session_id, tab_id) else {
            return;
        };
        let previous = data.row_count;
        data.row_count = RowCount::Counting;
        cx.notify();
        let counted = filters.clone();
        let task = self
            .runtime
            .spawn(async move { engine.count_rows(&table, &filters, Some(&columns)).await });
        cx.spawn(async move |this, cx| {
            let Ok(result) = task.await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                let Some(data) = this.data_tab_mut(session_id, tab_id) else {
                    return;
                };
                if data.row_count != RowCount::Counting || data.counted_filters != counted {
                    return;
                }
                match result {
                    Ok(count) => data.row_count = RowCount::Exact(count),
                    Err(error) => {
                        data.row_count = previous;
                        this.show_toast(ToastKind::Error, error.to_string(), cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}

/// `1234567` as `1,234,567`.
pub(super) fn grouped(count: u64) -> String {
    let digits = count.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

/// A short approximate figure such as `≈ 1.2M`.
pub(super) fn approximate(count: u64) -> String {
    let (value, suffix) = match count {
        0..1_000 => return format!("≈ {count}"),
        1_000..1_000_000 => (count as f64 / 1e3, "K"),
        1_000_000..1_000_000_000 => (count as f64 / 1e6, "M"),
        _ => (count as f64 / 1e9, "B"),
    };
    if value < 10. {
        format!("≈ {value:.1}{suffix}")
    } else {
        format!("≈ {value:.0}{suffix}")
    }
}

#[cfg(test)]
mod tests {
    use super::{approximate, grouped};

    #[test]
    fn counts_are_grouped_and_approximated() {
        assert_eq!(grouped(0), "0");
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(1_000), "1,000");
        assert_eq!(grouped(1_234_567), "1,234,567");
        assert_eq!(approximate(950), "≈ 950");
        assert_eq!(approximate(1_250), "≈ 1.2K");
        assert_eq!(approximate(48_000), "≈ 48K");
        assert_eq!(approximate(3_400_000), "≈ 3.4M");
        assert_eq!(approximate(7_000_000_000), "≈ 7.0B");
    }
}
