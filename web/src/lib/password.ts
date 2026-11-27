export const MIN_LENGTH = 12;
export const MAX_LENGTH = 256; // same limits as the server

const common = /(password|passw0rd|qwerty|123456|abcdef|admin|letmein|welcome|rahasia)/i;

export type Strength = {
	/** 0 empty, 1 weak, 2 fair, 3 strong, 4 very strong. Advice only: the server enforces length. */
	score: 0 | 1 | 2 | 3 | 4;
	rules: { length: boolean; mixed: boolean; variety: boolean; unpredictable: boolean };
};

export function strength(password: string): Strength {
	const classes = [/[a-z]/, /[A-Z]/, /\d/, /[^A-Za-z0-9]/].filter((re) => re.test(password)).length;
	const rules = {
		length: password.length >= MIN_LENGTH,
		mixed: /[a-z]/.test(password) && /[A-Z]/.test(password),
		variety: /\d/.test(password) && /[^A-Za-z0-9]/.test(password),
		unpredictable: password.length > 0 && !common.test(password) && new Set(password).size >= 5
	};
	let score = 0;
	if (password) {
		score = 1;
		if (rules.length && rules.unpredictable) score = 2 + Number(classes >= 3) + Number(password.length >= 16);
	}
	return { score: score as Strength['score'], rules };
}
