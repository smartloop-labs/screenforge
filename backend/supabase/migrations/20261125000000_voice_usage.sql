-- ElevenLabs usage (narration, voice enhance) per device and day, at list price. ElevenLabs is
-- free for everyone, so this is its only limit: the `voice` and `voice-enhance` functions cap
-- what one device (and all of them together) uses a day. Kept apart from `agent_usage`, so
-- narration never uses up the agent's daily caps. Keyed like it by a SHA-256 of the machine's
-- hardware UUID.
create table if not exists public.voice_usage (
  device_id text not null check (device_id ~ '^[0-9a-f]{64}$'),
  day date not null default (now() at time zone 'utc')::date,
  requests integer not null default 0 check (requests >= 0),
  cost_usd numeric(12, 6) not null default 0 check (cost_usd >= 0),
  updated_at timestamptz not null default now(),
  primary key (device_id, day)
);
create index if not exists voice_usage_day on public.voice_usage (day);
alter table public.voice_usage enable row level security; -- service role only

-- Today's use for one device and for everyone (UTC days).
create or replace function public.voice_usage_today(p_device text)
returns table (device_usd numeric, total_usd numeric)
language sql
stable
security definer
set search_path = public
as $$
  select
    coalesce(sum(cost_usd) filter (where device_id = p_device), 0),
    coalesce(sum(cost_usd), 0)
  from public.voice_usage
  where day = (now() at time zone 'utc')::date;
$$;

-- Add one request's cost to the device's row for today.
create or replace function public.voice_usage_add(p_device text, p_cost numeric)
returns void
language sql
security definer
set search_path = public
as $$
  insert into public.voice_usage (device_id, requests, cost_usd)
  values (p_device, 1, greatest(p_cost, 0))
  on conflict (device_id, day) do update
    set requests = public.voice_usage.requests + 1,
        cost_usd = public.voice_usage.cost_usd + greatest(p_cost, 0),
        updated_at = now();
$$;
revoke all on function public.voice_usage_today(text) from public, anon, authenticated;
revoke all on function public.voice_usage_add(text, numeric) from public, anon, authenticated;
