create table users (
  id text primary key,
  hca_sub text not null unique,
  email text, name text,
  verification_status text,
  handle text,
  display_name text,
  profile_public boolean not null default true,
  role text not null default 'user' check (role in ('user','reviewer','admin')),
  created_at timestamptz not null default now()
);
create unique index users_handle_lower on users (lower(handle));
create table sessions (
  token_hash text primary key,
  user_id text not null references users(id) on delete cascade,
  csrf text not null,
  expires_at timestamptz not null
);
create table oauth_states (
  state_hash text primary key,
  verifier text not null, nonce text not null, return_to text not null,
  expires_at timestamptz not null
);
