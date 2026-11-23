// The site's contact form (screenforge.studio/contact): { name, email, topic, message, website }
// is sent through Resend to support@screenforge.studio, with the visitor's address as reply-to
// so answering the email answers them. `website` is a honeypot field hidden from people; a
// filled one is accepted silently and dropped. Nothing is stored.
import { serve } from "https://deno.land/std@0.190.0/http/server.ts";

const TO = Deno.env.get("SUPPORT_EMAIL") ?? "support@screenforge.studio";
const TOPICS = ["General", "License or billing", "Bug report", "Feature request", "Press or partnership"];
const EMAIL = /^[^\s@<>]+@[^\s@<>]+\.[^\s@<>]+$/;

const cors = { "Access-Control-Allow-Origin": "*", "Access-Control-Allow-Headers": "authorization, x-client-info, apikey, content-type" };
const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { ...cors, "Content-Type": "application/json" } });
const clean = (v: unknown, max: number) => (typeof v === "string" ? v.replace(/\r\n?/g, "\n").trim().slice(0, max) : "");
// header-bound values must stay on one line
const line = (s: string) => s.replace(/[\r\n]+/g, " ");

serve(async (req) => {
  if (req.method === "OPTIONS") return new Response(null, { headers: cors });
  if (req.method !== "POST") return json({ error: "POST only" }, 405);
  const body = await req.json().catch(() => ({}));
  if (clean(body.website, 200)) return json({ ok: true });

  const name = line(clean(body.name, 100));
  const email = line(clean(body.email, 200));
  const topic = TOPICS.includes(body.topic) ? body.topic : "General";
  const message = clean(body.message, 5000);
  if (!EMAIL.test(email)) return json({ error: "Enter a valid email address." }, 400);
  if (message.length < 10) return json({ error: "Write a little more so we can help." }, 400);

  const api = Deno.env.get("RESEND_API_KEY");
  if (!api) { console.error("RESEND_API_KEY not set; contact message dropped"); return json({ error: "Mail is not set up" }, 500); }
  const r = await fetch("https://api.resend.com/emails", {
    method: "POST",
    headers: { Authorization: `Bearer ${api}`, "Content-Type": "application/json" },
    body: JSON.stringify({
      from: Deno.env.get("CONTACT_FROM_EMAIL") ?? "ScreenForge Contact <support@screenforge.studio>",
      to: [TO],
      reply_to: name ? `${name.replace(/[<>"]/g, "")} <${email}>` : email,
      subject: `[${topic}] ${name || email}`,
      text: `From: ${name || "(no name)"} <${email}>\nTopic: ${topic}\n\n${message}\n`,
    }),
  });
  if (!r.ok) { console.error("resend", r.status, await r.text()); return json({ error: "Could not send" }, 502); }
  return json({ ok: true });
});
