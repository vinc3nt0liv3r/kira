ALTER TABLE events ADD COLUMN category_type text;

-- Backfill rows collected before the typed column existed so historical
-- events expose http_category_type through category_type without an attrs
-- lookup on every query.
UPDATE events
SET category_type = attrs->>'http_category_type'
WHERE category_type IS NULL
  AND attrs ? 'http_category_type'
  AND COALESCE(attrs->>'http_category_type', '') <> '';

CREATE INDEX IF NOT EXISTS events_category_type_idx
    ON events (category_type) WHERE category_type IS NOT NULL;
