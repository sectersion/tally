-- One claim per action_id per chain (spec: ids are unique), so a claim cannot be re-chained to multiply hours.
create unique index ledger_entries_chain_action on ledger_entries (chain_id, action_id);
