import type { Destination } from './api/http.ts';

export type Kind = Destination['kind'];
export const kinds: Record<Kind, { label: string; brand: string; port: number }> = {
	s3: { label: 'S3 / MinIO', brand: 'S3 / MinIO', port: 0 },
	sftp: { label: 'SFTP / SSH', brand: 'SFTP / SSH', port: 22 },
	ftp: { label: 'FTP / FTPS', brand: 'FTP / FTPS', port: 21 },
	smb: { label: 'Samba / SMB', brand: 'Samba', port: 445 }
};

export const blankStorage = (kind: Kind = 's3') => ({
	name: '', agent: '', kind: kind as string,
	endpoint: '', region: '', bucket: '', prefix: '', access_key: '', secret_key: '', virtual_host: false, public_endpoint: '',
	host: '', port: kinds[kind].port, username: '', password: '', auth: 'password', private_key: '', passphrase: '', fingerprint: '', domain: '', tls: false
});
export type StorageForm = ReturnType<typeof blankStorage>;

/** The current non-secret values of a stored destination. */
export function formOf(d: Destination): StorageForm {
	const form = { ...blankStorage(d.kind), name: d.name, agent: d.agent_id, bucket: d.bucket, prefix: d.prefix };
	if (d.kind === 's3') {
		return { ...form, endpoint: d.endpoint, region: d.region ?? '', virtual_host: d.virtual_host ?? false, public_endpoint: d.public_endpoint ?? '' };
	}
	// `scheme://host:port`; URL() would drop a default port.
	const match = /^(\w+):\/\/(.*):(\d+)$/.exec(d.endpoint);
	return { ...form, host: match?.[2] ?? '', port: Number(match?.[3] ?? kinds[d.kind].port), tls: match?.[1] === 'ftps', username: d.username ?? '' };
}

/** What the server lists (`record`) and what only the agent may read (`secret`). */
export function connectionOf(form: StorageForm) {
	const kind = form.kind as Kind;
	const prefix = form.prefix.trim();
	if (kind === 's3') {
		const shared = { endpoint: form.endpoint.trim(), region: form.region.trim(), bucket: form.bucket.trim(), prefix, virtual_host: form.virtual_host, public_endpoint: form.public_endpoint.trim() };
		return { record: shared, secret: { kind, ...shared, access_key: form.access_key, secret_key: form.secret_key } };
	}
	const host = form.host.trim();
	const username = form.username.trim();
	const bucket = kind === 'smb' ? form.bucket.trim() : '';
	const scheme = kind === 'ftp' && form.tls ? 'ftps' : kind;
	const key = kind === 'sftp' && form.auth === 'key';
	return {
		record: { endpoint: `${scheme}://${host}:${form.port}`, bucket, prefix, username },
		secret: {
			kind, host, port: form.port, username, bucket, prefix,
			password: key ? '' : form.password,
			private_key: key ? form.private_key : '',
			passphrase: key ? form.passphrase : '',
			fingerprint: kind === 'sftp' ? form.fingerprint.trim() : '',
			domain: kind === 'smb' ? form.domain.trim() : '',
			tls: kind === 'ftp' && form.tls
		}
	};
}
