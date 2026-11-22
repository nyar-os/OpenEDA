/**
 * Stable error surface of `@wae/open-eda`.
 *
 * Codes are contract, not prose: callers compare them, localize around them and
 * never parse the message. A failure therefore keeps its code even when the
 * text is translated.
 */

/** Every stable code the authoring surface may report. */
export const errorCodes = [
    'session/disposed',
    'session/argument-invalid',
    'session/budget-exceeded',
    'design/not-found',
    'design/revision-conflict',
    'object/not-found',
    'object/unsupported',
    'object/duplicate',
    'history/empty',
    'junction/duplicate',
    'junction/not-found',
    'no-connect/duplicate',
    'no-connect/pin-missing',
    'no-connect/not-found',
    'sheet/not-found',
    'sheet/last-sheet',
    'sheet/not-empty',
    'format/unsupported',
    'format/unsupported-version',
    'format/invalid',
    'capability/not-implemented',
] as const;

/** Union of {@link errorCodes}. */
export type OpenEDAErrorCode = (typeof errorCodes)[number];

/** Optional locators a failure can carry so callers can point at the object. */
export interface OpenEDAErrorDetails {
    readonly revision?: number;
    readonly sheetId?: string;
    readonly objectId?: string;
    /** Design format involved in the failure, without the leading dot. */
    readonly format?: string;
    /** Declared capability involved in the failure, e.g. `design.export`. */
    readonly feature?: string;
}

/** Options accepted by {@link OpenEDAError}. */
export interface OpenEDAErrorOptions {
    readonly recoverable?: boolean;
    readonly details?: OpenEDAErrorDetails;
    readonly cause?: unknown;
}

/** Every failure thrown or rejected by this package. */
export class OpenEDAError extends Error {
    /** Stable, comparable code. */
    readonly code: OpenEDAErrorCode;
    /** Whether retrying with different input or state can succeed. */
    readonly recoverable: boolean;
    /** Optional locators (`revision`, `sheetId`, `objectId`, `format`, `feature`). */
    readonly details: OpenEDAErrorDetails;

    constructor(code: OpenEDAErrorCode, message: string, options: OpenEDAErrorOptions = {}) {
        super(message, options.cause === undefined ? undefined : { cause: options.cause });
        this.name = 'OpenEDAError';
        this.code = code;
        this.recoverable = options.recoverable ?? false;
        this.details = Object.freeze({ ...options.details });
    }
}

/**
 * Reported when the native addon is absent or when a declared capability is not
 * implemented by the installed build. Supported document, inspection and export
 * operations never use this code.
 */
export class NotImplementedError extends OpenEDAError {
    constructor(feature: string, message = `\`${feature}\` is not implemented yet`) {
        super('capability/not-implemented', message, { recoverable: false, details: { feature } });
        this.name = 'NotImplementedError';
    }
}

/** Narrows an unknown thrown value to {@link OpenEDAError}. */
export function isOpenEDAError(value: unknown): value is OpenEDAError {
    return value instanceof OpenEDAError;
}
