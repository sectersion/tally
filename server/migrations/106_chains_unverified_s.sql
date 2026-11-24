-- Maintained alongside chains.verified_s so public reads need not sum every ledger row.
alter table chains add column unverified_s bigint not null default 0;
update chains c set unverified_s = coalesce((select sum(e.unverified_s) from ledger_entries e where e.chain_id = c.id), 0);
