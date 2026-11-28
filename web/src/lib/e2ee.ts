// Browser side of crates/shared/src/e2ee.rs. Pure functions (no app imports) so that
// scripts/e2ee.interop.ts can check them against the Rust server from Node.
export const HEADER = 'x-dbm-e2ee';
export const CONTENT_TYPE = 'application/dbm-e2ee';
/** A response that never ends: a 32 byte seed, then length-prefixed sealed chunks. */
export const STREAM_CONTENT_TYPE = 'application/dbm-e2ee-stream';

/** Sent by a side that can read gzip bodies; without it nothing is compressed for it. */
export const GZIP_HEADER = 'x-dbm-gzip';
/** Smaller bodies gain little, and passwords and tokens travel in small ones. */
const GZIP_MIN = 1024;
/** Set in the status or the authorization length, inside the ciphertext: the body is gzip. */
const GZIPPED = 0x8000;

const KEY_ID = 1;
// KEM 0x0020 (X25519), KDF 0x0001 (HKDF-SHA256), AEAD 0x0002 (AES-256-GCM)
const SUITE = [0x00, 0x20, 0x00, 0x01, 0x00, 0x02];

type Bytes = Uint8Array<ArrayBuffer>;

const text = (value: string): Bytes => new TextEncoder().encode(value) as Bytes;

function concat(...parts: (Uint8Array | number[])[]): Bytes {
	const out = new Uint8Array(parts.reduce((sum, p) => sum + p.length, 0));
	let at = 0;
	for (const part of parts) {
		out.set(part, at);
		at += part.length;
	}
	return out;
}

export function encode(bytes: Uint8Array): string {
	let binary = '';
	for (const byte of bytes) binary += String.fromCharCode(byte);
	return btoa(binary).replaceAll('+', '-').replaceAll('/', '_').replaceAll('=', '');
}

export function decode(value: string): Bytes {
	const binary = atob(value.replaceAll('-', '+').replaceAll('_', '/'));
	return Uint8Array.from(binary, (c) => c.charCodeAt(0));
}

const through = async (data: Bytes, transform: CompressionStream | DecompressionStream): Promise<Bytes> =>
	new Uint8Array(await new Response(new Blob([data]).stream().pipeThrough(transform)).arrayBuffer());

/** Length prefixes keep field boundaries unambiguous. */
function aad(...fields: string[]): Bytes {
	return concat(
		...fields.flatMap((field) => {
			const bytes = text(field);
			const length = new Uint8Array(4);
			new DataView(length.buffer).setUint32(0, bytes.length);
			return [length, bytes];
		})
	);
}

// Loaded on demand: nothing is downloaded when the server has encryption off.
async function sender(publicKey: Bytes, info: Bytes) {
	const { Aes256Gcm, CipherSuite, DhkemX25519HkdfSha256, HkdfSha256 } = await import('@hpke/core');
	const suite = new CipherSuite({ kem: new DhkemX25519HkdfSha256(), kdf: new HkdfSha256(), aead: new Aes256Gcm() });
	const recipientPublicKey = await suite.kem.deserializePublicKey(publicKey.buffer);
	return suite.createSenderContext({ recipientPublicKey, info });
}

async function derive(secret: ArrayBuffer, salt: Bytes, label: string, bytes: number): Promise<ArrayBuffer> {
	const key = await crypto.subtle.importKey('raw', secret, 'HKDF', false, ['deriveBits']);
	return crypto.subtle.deriveBits({ name: 'HKDF', hash: 'SHA-256', salt, info: text(label) }, key, bytes * 8);
}

export type Sealed = {
	header: string;
	/** Present for requests with a body; otherwise the proof is inside the header. */
	body?: Bytes;
	open: (reply: ArrayBuffer) => Promise<{ status: number; body: string }>;
	/** Opens a stream chunk by chunk; call the result with the network pieces as they arrive. */
	stream: () => (piece: Uint8Array) => Promise<string[]>;
};

export async function sealRequest(
	serverKey: string,
	method: string,
	pathAndQuery: string,
	body: string,
	timestamp = Math.floor(Date.now() / 1000),
	gzip = false
): Promise<Sealed> {
	const context = await sender(decode(serverKey), concat(text('dbm/v1 request\0'), [KEY_ID], SUITE));
	const enc = new Uint8Array(context.enc) as Bytes;
	let content = text(body);
	let flag = 0;
	if (gzip && content.length >= GZIP_MIN) {
		const small = await through(content, new CompressionStream('gzip'));
		if (small.length < content.length) [content, flag] = [small, GZIPPED];
	}
	// Plaintext: authorization length (0, the browser uses cookies), then the body.
	const sealed = new Uint8Array(
		await context.seal(concat([flag >> 8, 0], content), aad(method, pathAndQuery, String(timestamp)))
	) as Bytes;
	const secret = await context.export(text('dbm/v1 response'), 32);
	const inHeader = method === 'GET';

	async function cipher(seed: Bytes, keyLabel: string, nonceLabel: string) {
		const salt = concat(enc, seed);
		const key = await crypto.subtle.importKey('raw', await derive(secret, salt, keyLabel, 32), 'AES-GCM', false, ['decrypt']);
		return { key, iv: new Uint8Array(await derive(secret, salt, nonceLabel, 12)) as Bytes };
	}

	return {
		header: `1.${timestamp}.${encode(enc)}${inHeader ? `.${encode(sealed)}` : ''}`,
		body: inHeader ? undefined : sealed,
		stream() {
			let buffer: Bytes = new Uint8Array(0);
			let state: Awaited<ReturnType<typeof cipher>> | undefined;
			let counter = 0;
			return async (piece) => {
				buffer = concat(buffer, piece);
				if (!state) {
					if (buffer.length < 32) return [];
					state = await cipher(buffer.slice(0, 32), 'stream key', 'stream nonce');
					buffer = buffer.slice(32);
				}
				const messages: string[] = [];
				while (buffer.length >= 4) {
					const end = 4 + new DataView(buffer.buffer, buffer.byteOffset).getUint32(0);
					if (end > 4096) throw new Error('chunk too large');
					if (buffer.length < end) break;
					// The chunk number is part of the nonce, so a missing or moved chunk fails here.
					const iv = state.iv.slice();
					const view = new DataView(iv.buffer);
					view.setUint32(8, (view.getUint32(8) ^ counter++) >>> 0);
					const plain = await crypto.subtle.decrypt({ name: 'AES-GCM', iv }, state.key, buffer.slice(4, end));
					messages.push(new TextDecoder().decode(plain));
					buffer = buffer.slice(end);
				}
				return messages;
			};
		},
		async open(reply) {
			const { key, iv } = await cipher(new Uint8Array(reply.slice(0, 32)), 'key', 'nonce');
			const plain = await crypto.subtle.decrypt({ name: 'AES-GCM', iv }, key, new Uint8Array(reply, 32));
			const field = new DataView(plain).getUint16(0);
			let content: Bytes = new Uint8Array(plain, 2);
			if (field & GZIPPED) content = await through(content, new DecompressionStream('gzip'));
			return { status: field & ~GZIPPED, body: new TextDecoder().decode(content) };
		}
	};
}

/** Everything a sealed secret is tied to; the agent must be given the same values. */
export type SecretContext = { agentId: string; secretId: string; kind: string; version: number };

/** Seals to an agent's public key; the server can store the result but not read it. */
export async function sealSecret(agentKey: string, context: SecretContext, plain: string): Promise<string> {
	const hpke = await sender(decode(agentKey), text('dbm/v1 agent secret'));
	const sealed = await hpke.seal(
		text(plain),
		aad(context.agentId, context.secretId, context.kind, String(context.version))
	);
	return encode(concat(new Uint8Array(hpke.enc), new Uint8Array(sealed)));
}
