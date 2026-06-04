-- Track whether each node currently reports itself ready to serve traffic.
--
-- Readiness is self-reported by each node on every heartbeat (see the heartbeat
-- task). Node discovery uses it to advertise only ready followers to the proxy.
-- Nodes default to not-ready until their first successful readiness report, so a
-- freshly registered node is never advertised before it has confirmed readiness.
ALTER TABLE nodes
ADD COLUMN ready BOOLEAN NOT NULL DEFAULT FALSE;
