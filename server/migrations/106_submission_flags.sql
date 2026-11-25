-- Reviewer triage (issue #113): behaviour flags computed at upload. Advisory, never changes hours.
alter table submissions add column flags jsonb not null default '[]', add column risk int not null default 0;
