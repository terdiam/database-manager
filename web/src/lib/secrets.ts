import { ApiError, type Agent } from './api/http.ts';
import { sealSecret } from './e2ee.ts';

/** Seals a secret to the agent that will use it: the server stores it but cannot read it. */
export async function sealFor(agent: Agent | undefined, kind: 'database' | 'destination' | 'ssh', id: string, secret: object) {
	if (!window.isSecureContext) throw new ApiError('insecure_context');
	if (!agent?.key) throw new ApiError('agent');
	return sealSecret(agent.key, { agentId: agent.id, secretId: id, kind, version: 1 }, JSON.stringify(secret));
}
