alter table projects
  add constraint projects_owner_fk foreign key (owner_id) references users(id),
  add column public boolean not null default true,
  add column upd_record_id text,
  add column change_seq bigint not null default 0;
create unique index projects_public_name_lower on projects (lower(public_name));
