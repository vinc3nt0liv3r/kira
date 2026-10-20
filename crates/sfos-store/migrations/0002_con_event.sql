-- SFOS logs firewall connections twice (`con_event=Start` and `Stop`). Keeping
-- the marker on the row lets dashboards count each connection once while the
-- event explorer still shows both records.
ALTER TABLE events ADD COLUMN IF NOT EXISTS con_event text;
