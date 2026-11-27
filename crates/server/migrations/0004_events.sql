-- Tell the server which table changed, so open pages can reload it. Only the table name is sent.
CREATE FUNCTION notify_change() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    PERFORM pg_notify('dbm_events', TG_TABLE_NAME);
    RETURN NULL;
END $$;

-- Row triggers: an update that matches nothing stays silent. Identical notifications
-- within one transaction are merged by PostgreSQL.
CREATE TRIGGER notify_change AFTER INSERT OR UPDATE OR DELETE ON users FOR EACH ROW EXECUTE FUNCTION notify_change();
CREATE TRIGGER notify_change AFTER INSERT OR UPDATE OR DELETE ON agents FOR EACH ROW EXECUTE FUNCTION notify_change();
CREATE TRIGGER notify_change AFTER INSERT ON audit_logs FOR EACH ROW EXECUTE FUNCTION notify_change();
CREATE TRIGGER notify_change AFTER INSERT OR UPDATE OR DELETE ON database_servers FOR EACH ROW EXECUTE FUNCTION notify_change();
CREATE TRIGGER notify_change AFTER INSERT OR UPDATE OR DELETE ON destinations FOR EACH ROW EXECUTE FUNCTION notify_change();
CREATE TRIGGER notify_change AFTER INSERT OR UPDATE OR DELETE ON backup_configs FOR EACH ROW EXECUTE FUNCTION notify_change();
CREATE TRIGGER notify_change AFTER INSERT OR UPDATE OR DELETE ON jobs FOR EACH ROW EXECUTE FUNCTION notify_change();
CREATE TRIGGER notify_change AFTER INSERT OR UPDATE OR DELETE ON snapshots FOR EACH ROW EXECUTE FUNCTION notify_change();
