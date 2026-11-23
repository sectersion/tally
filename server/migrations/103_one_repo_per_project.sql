-- One live GitHub repo per project, so the page and unlink-repo mean one thing.
create unique index project_repos_one_per_project on project_repos (project_id) where unlinked_at is null;
