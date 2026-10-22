UPDATE event_rollup_state
SET history_ready = true, updated_at = now()
WHERE singleton
  AND NOT history_ready;
