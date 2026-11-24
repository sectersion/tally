-- repository.updated_at of the newest rename/transfer applied; GitHub does not deliver webhooks in order.
alter table github_repos add column gh_updated_at timestamptz;
