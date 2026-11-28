// Reads what the audit log keeps about a caller. Pure, so scripts/client.check.ts can run it.

export type Network = 'local' | 'private' | 'public' | 'console' | 'system' | 'unknown';

export function network(ip: string): Network {
	if (ip === 'console' || ip === 'system') return ip;
	if (ip === '::1' || ip.startsWith('127.')) return 'local';
	if (/^(10\.|192\.168\.|169\.254\.|172\.(1[6-9]|2\d|3[01])\.|f[cd][0-9a-f]{2}:|fe80:)/i.test(ip)) return 'private';
	return /^[\d.]+$|:/.test(ip) ? 'public' : 'unknown';
}

// Order matters: Edge and Opera also say Chrome, and Chrome also says Safari.
const BROWSERS: [RegExp, string][] = [
	[/Edg(?:e|A|iOS)?\/(\d+)/, 'Edge'],
	[/OPR\/(\d+)/, 'Opera'],
	[/(?:Firefox|FxiOS)\/(\d+)/, 'Firefox'],
	[/(?:Chrome|CriOS)\/(\d+)/, 'Chrome'],
	[/Version\/(\d+).*Safari\//, 'Safari'],
	[/^curl\/(\d+)/, 'curl']
];
const SYSTEMS: [RegExp, string][] = [
	[/Android/, 'Android'],
	[/iPhone|iPad/, 'iOS'],
	[/Windows/, 'Windows'],
	[/Mac OS X/, 'macOS'],
	[/Linux/, 'Linux']
];

/** "Chrome 140 · macOS", or the text itself when it is nothing known. Empty stays empty. */
export function device(agent: string): string {
	const system = SYSTEMS.find(([pattern]) => pattern.test(agent))?.[1];
	for (const [pattern, name] of BROWSERS) {
		const version = pattern.exec(agent)?.[1];
		if (version) return [`${name} ${version}`, system].filter(Boolean).join(' · ');
	}
	return agent.length > 40 ? `${agent.slice(0, 40)}…` : agent;
}
