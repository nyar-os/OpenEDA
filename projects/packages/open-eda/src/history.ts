/**
 * Sidecar undo history for `new OpenEDA({ history: 'sidecar' })`.
 *
 * The native model owns a bounded history per open handle, but a CLI invocation
 * is its own process and gets a fresh handle. In sidecar mode the session keeps
 * the document it is about to replace in `<file>.history.json`, so `undo` and
 * `redo` survive separate invocations.
 *
 * This module is only loaded through a dynamic `import('./history.js')`, so a
 * plain `history: 'memory'` session never touches `node:fs`.
 */

import fs from 'node:fs';

/** Snapshots kept per sidecar. */
export const HISTORY_LIMIT = 32;
/** Native document envelope written when a session materialises an import. */
export const DOCUMENT_FORMAT = 'open-eda-design';
/** Current envelope version. */
export const DOCUMENT_VERSION = 1;

const HISTORY_VERSION = 1;

/** Shape of `<file>.history.json`. Snapshots are whole document files. */
export interface DocumentHistory {
    readonly version: number;
    undo: string[];
    redo: string[];
}

/** Number of snapshots available in each direction. */
export interface HistoryCounts {
    readonly undo: number;
    readonly redo: number;
}

/** Result of swapping one snapshot back into the document. */
export interface StepResult extends HistoryCounts {
    readonly revision: number;
}

/** Path of the sidecar that belongs to one document. */
export function historyPath(file: string): string {
    return `${file}.history.json`;
}

/** Reads the sidecar, or returns an empty history when it is absent or foreign. */
export function readHistory(file: string): DocumentHistory {
    try {
        const parsed = JSON.parse(fs.readFileSync(historyPath(file), 'utf8')) as Partial<DocumentHistory>;
        if (parsed.version === HISTORY_VERSION && Array.isArray(parsed.undo) && Array.isArray(parsed.redo)) {
            return {
                version: HISTORY_VERSION,
                undo: parsed.undo.filter((item): item is string => typeof item === 'string'),
                redo: parsed.redo.filter((item): item is string => typeof item === 'string'),
            };
        }
    } catch {
        // No history yet, or a file this build cannot read.
    }
    return { version: HISTORY_VERSION, undo: [], redo: [] };
}

/** Writes a file through a temporary path and a rename. */
export function writeAtomic(target: string, content: string): void {
    const temporary = `${target}.${process.pid}.tmp`;
    fs.writeFileSync(temporary, content);
    fs.renameSync(temporary, target);
}

function writeHistory(file: string, history: DocumentHistory): void {
    writeAtomic(historyPath(file), `${JSON.stringify(history)}\n`);
}

/** Removes the sidecar of one document; missing files are ignored. */
export function clearHistory(file: string): void {
    fs.rmSync(historyPath(file), { force: true });
}

/** Reads the document content a mutation is about to replace. */
export function readDocument(file: string): string | undefined {
    try {
        return fs.readFileSync(file, 'utf8');
    } catch {
        return undefined;
    }
}

/** Number of snapshots available for one document. */
export function counts(file: string): HistoryCounts {
    const history = readHistory(file);
    return { undo: history.undo.length, redo: history.redo.length };
}

/** Pushes the replaced document and clears the redo direction. */
export function recordStep(file: string, previous: string): HistoryCounts {
    const history = readHistory(file);
    history.undo.push(previous);
    if (history.undo.length > HISTORY_LIMIT) {
        history.undo.splice(0, history.undo.length - HISTORY_LIMIT);
    }
    history.redo.length = 0;
    writeHistory(file, history);
    return { undo: history.undo.length, redo: history.redo.length };
}

/**
 * Swaps the newest snapshot in one direction back into the document.
 *
 * The restored revision is bumped relative to the current file, matching the
 * native per-handle history, so revisions stay monotonic across moves. Returns
 * `undefined` when that direction holds no snapshot.
 */
export function stepBack(file: string, direction: 'undo' | 'redo'): StepResult | undefined {
    const history = readHistory(file);
    const source = direction === 'undo' ? history.undo : history.redo;
    const destination = direction === 'undo' ? history.redo : history.undo;
    const snapshot = source.pop();
    if (snapshot === undefined) {
        return undefined;
    }
    const current = fs.readFileSync(file, 'utf8');
    const revision = revisionOf(current) + 1;
    const restored = JSON.parse(snapshot) as { document: { revision: number } };
    restored.document.revision = revision;
    writeAtomic(file, `${JSON.stringify(restored, null, 2)}\n`);
    destination.push(current);
    if (destination.length > HISTORY_LIMIT) {
        destination.splice(0, destination.length - HISTORY_LIMIT);
    }
    writeHistory(file, history);
    return { revision, undo: history.undo.length, redo: history.redo.length };
}

/** Reads the committed revision from a document this session wrote. */
export function revisionOf(document: string): number {
    const parsed = JSON.parse(document) as { document?: { revision?: unknown } };
    const revision = parsed.document?.revision;
    return typeof revision === 'number' ? revision : 1;
}

/** Writes an imported design as a native document under the current envelope. */
export function writeDocument(target: string, document: unknown): void {
    writeAtomic(
        target,
        `${JSON.stringify({ format: DOCUMENT_FORMAT, version: DOCUMENT_VERSION, document }, null, 2)}\n`,
    );
}
