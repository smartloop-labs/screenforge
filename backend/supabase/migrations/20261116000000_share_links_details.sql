-- What the page says about a shared video, from the export's details: a description (which may
-- end with the chapter list, `00:00 Intro` lines) and tags.
alter table public.share_links add column if not exists description text;
alter table public.share_links add column if not exists tags text[];
