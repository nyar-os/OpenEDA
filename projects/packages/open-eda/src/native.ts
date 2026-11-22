/**
 * Native addon discovery.
 *
 * The addon only maps values: design semantics, inspection and export stay in
 * the `open-eda-model` crate. An absent addon is a declarative state, so calls
 * report `capability/not-implemented` instead of inventing a design.
 */

import { createRequire } from 'node:module';

const requireNative = createRequire(import.meta.url);

/** Orthogonal exports of the built addon. Every call returns a JSON string. */
export interface NativeBinding {
    version(): string;
    capabilities(): string;
    design_create(requestJson: string): string;
    design_open(path: string): string;
    design_save(handle: number, path: string, overwrite: boolean): string;
    design_set_properties(handle: number, requestJson: string): string;
    design_set_pin_type(handle: number, requestJson: string): string;
    design_annotate(handle: number): string;
    design_inspect(handle: number, checksJson?: string): string;
    design_inspect_with(handle: number, optionsJson: string): string;
    design_export(handle: number, format: string, path: string, variant?: string): string;
    import_kicad_sch(source: string): string;
    import_kicad_symbol(source: string): string;
    import_kicad_net(source: string): string;
    design_compare_netlist(handle: number, source: string): string;
    diff_documents(left: string, right: string): string;
    design_close(handle: number): boolean;
    design_summary(handle: number): string;
    design_list(handle: number): string;
    design_symbols(handle: number): string;
    design_connectivity(handle: number): string;
    design_add_component(handle: number, requestJson: string): string;
    design_copy_component(handle: number, requestJson: string): string;
    design_delete_component(handle: number, reference: string, sheet?: string): string;
    design_connect_net(handle: number, requestJson: string): string;
    design_add_wire(handle: number, requestJson: string): string;
    design_update_wire(handle: number, requestJson: string): string;
    design_split_wire(handle: number, requestJson: string): string;
    design_translate(handle: number, requestJson: string): string;
    design_list_wires(handle: number, sheet?: string): string;
    design_add_junction(handle: number, requestJson: string): string;
    design_add_text(handle: number, requestJson: string): string;
    design_add_no_connect(handle: number, requestJson: string): string;
    design_delete_no_connect(handle: number, index: number, sheet?: string): string;
    design_list_components(handle: number, sheet?: string): string;
    design_list_nets(handle: number, sheet?: string): string;
    design_add_bus(handle: number, requestJson: string): string;
    design_remove_bus(handle: number, bus: string): string;
    design_list_buses(handle: number): string;
    design_add_net_class(handle: number, requestJson: string): string;
    design_remove_net_class(handle: number, netClass: string): string;
    design_list_net_classes(handle: number): string;
    design_set_reference_prefix(handle: number, requestJson: string): string;
    design_remove_reference_prefix(handle: number, componentClass: string): string;
    design_list_reference_prefixes(handle: number): string;
    design_set_component_class(handle: number, requestJson: string): string;
    design_list_component_classes(handle: number): string;
    design_add_variant(handle: number, requestJson: string): string;
    design_remove_variant(handle: number, name: string): string;
    design_list_variants(handle: number): string;
    design_undo(handle: number): string;
    design_redo(handle: number): string;
    design_add_sheet(handle: number, name?: string): string;
    design_rename_sheet(handle: number, sheet: string, name: string): string;
    design_delete_sheet(handle: number, sheet: string): string;
    design_add_hierarchy(handle: number, requestJson: string): string;
    design_list_hierarchy(handle: number): string;
    design_remove_hierarchy(handle: number, sheet: string): string;
    design_add_hierarchy_port(handle: number, requestJson: string): string;
    design_remove_hierarchy_port(handle: number, requestJson: string): string;
    design_move_component(handle: number, requestJson: string): string;
    design_update_component(handle: number, requestJson: string): string;
    design_set_component_footprint(handle: number, requestJson: string): string;
    design_set_component_property(handle: number, requestJson: string): string;
    design_remove_component_property(handle: number, requestJson: string): string;
    design_list_component_properties(handle: number, reference: string, sheet?: string): string;
    design_rename_net(handle: number, requestJson: string): string;
    design_merge_nets(handle: number, requestJson: string): string;
    design_delete_wire(handle: number, wire: string, sheet?: string): string;
    design_derive_nets(handle: number, sheet?: string): string;
    design_disconnect_net(handle: number, requestJson: string): string;
    design_delete_net(handle: number, requestJson: string): string;
    design_delete_junction(handle: number, index: number, sheet?: string): string;
    design_move_junction(handle: number, index: number, sheet: string | undefined, x: number, y: number): string;
    design_delete_text(handle: number, index: number, sheet?: string): string;
    design_move_text(handle: number, index: number, sheet: string | undefined, x: number, y: number): string;
    design_list_texts(handle: number, sheet?: string): string;
    design_update_text(handle: number, requestJson: string): string;
    openGui(options?: { title?: string; contentDir?: string; width?: number; height?: number }): string;
    pumpEvents(): string;
    closeGui(): string;
    isGuiOpen(): boolean;
}

let cached: NativeBinding | null | undefined;

/** Loads the addon once, or returns `undefined` when it is not installed. */
export function nativeBinding(): NativeBinding | undefined {
    if (cached === undefined) {
        cached = load();
    }
    return cached ?? undefined;
}

function load(): NativeBinding | null {
    for (const candidate of candidates()) {
        try {
            return requireNative(candidate) as NativeBinding;
        } catch (error) {
            if (candidate === explicitPath()) {
                throw error;
            }
            if (!isModuleNotFound(error, candidate)) {
                throw error;
            }
        }
    }
    return null;
}

function candidates(): string[] {
    const explicit = explicitPath();
    if (explicit) {
        return [explicit];
    }
    return [`@wae/open-eda-${process.platform}-${process.arch}`];
}

function explicitPath(): string | undefined {
    const value = process.env.OPEN_EDA_NATIVE;
    return value !== undefined && value.length > 0 ? value : undefined;
}

function isModuleNotFound(error: unknown, candidate: string): boolean {
    if (typeof error !== 'object' || error === null) {
        return false;
    }
    const code = (error as { code?: unknown }).code;
    // Node prints the resolved path, which uses the platform separator on
    // Windows. Normalize it so a linked platform package without a built
    // addon is still treated as "not installed" instead of a fatal error.
    const message = (error as Error).message.replace(/\\/g, '/');
    return code === 'MODULE_NOT_FOUND' && message.includes(candidate);
}
