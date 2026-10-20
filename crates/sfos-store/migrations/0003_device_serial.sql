-- SFOS 21+ sends `device_serial_id` instead of `device_id`. The parser now maps
-- both onto device_id; backfill rows collected before that fix so the collector
-- panel and per-device filters see the serial.
UPDATE events
SET device_id = attrs->>'device_serial_id'
WHERE device_id IS NULL
  AND attrs ? 'device_serial_id'
  AND COALESCE(attrs->>'device_serial_id', '') <> '';
