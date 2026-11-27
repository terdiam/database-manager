-- Non-secret copies of what is inside a sealed secret, so an edit form can show the current
-- values. Agents still use only the sealed ones. NULL for records made before this.
ALTER TABLE database_servers ADD COLUMN ssl_mode text;
ALTER TABLE destinations
    ADD COLUMN region          text,
    ADD COLUMN virtual_host    boolean,
    ADD COLUMN public_endpoint text;
