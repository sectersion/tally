-- One live GitHub repo per project, so the page and unlink-repo mean one thing.
-- Schema 102 allowed several; keep the newest live link per project and unlink the rest so the index can build.
update project_repos set unlinked_at = now()
where unlinked_at is null and (project_id, github_repo_id, linked_at) not in (
  select distinct on (project_id) project_id, github_repo_id, linked_at
  from project_repos where unlinked_at is null order by project_id, linked_at desc, github_repo_id);
create unique index project_repos_one_per_project on project_repos (project_id) where unlinked_at is null;
