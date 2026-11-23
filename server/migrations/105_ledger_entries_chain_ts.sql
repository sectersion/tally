-- commitWindows sums each witnessed push's window as a range on (chain_id, ts).
-- 104 is reserved for plan 10 (104_submissions.sql).
create index ledger_entries_chain_ts on ledger_entries (chain_id, ts);
