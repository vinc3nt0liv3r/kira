UPDATE event_rollup_state
SET history_ready = true, updated_at = now()
WHERE singleton
  AND NOT history_ready
  AND NOT EXISTS (SELECT 1 FROM events)
  AND NOT EXISTS (SELECT 1 FROM event_rollups);
