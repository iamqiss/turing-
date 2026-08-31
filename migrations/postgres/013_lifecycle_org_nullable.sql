-- Platform node events may have no org
ALTER TABLE lifecycle_events
    ALTER COLUMN org_id DROP NOT NULL;
