import { ChevronDown, ChevronRight, CopyPlus, FolderPlus, Layers3, Plus, Trash2 } from "lucide-react";
import { useEffect, useState } from "react";
import { api } from "../lib/bridge";
import type { Asset, AssetFilter, CatalogueCollection, CatalogueVersion, CollectionSet, SmartRule } from "../types";

type Props = { filter: AssetFilter; onFilter: (patch: Partial<AssetFilter>) => void; selectedIds: ReadonlySet<string>; active: Asset | null; onChanged: () => Promise<void> | void; onNotice: (message: string) => void };
type NameAction =
  | { kind: "createCollection"; setId?: string }
  | { kind: "createSet" }
  | { kind: "renameCollection"; collection: CatalogueCollection }
  | { kind: "renameSet"; set: CollectionSet }
  | { kind: "createVersion"; mode: "current" | "default" | "duplicate" }
  | { kind: "renameVersion"; version: CatalogueVersion };
const fields: SmartRule["field"][] = ["rating", "flag", "edited", "fileType", "keyword", "captureDate", "importDate", "camera", "lens", "versionStatus", "hasMultipleVersions", "stackStatus"];
const operators = (field: SmartRule["field"]) => field === "rating" ? ["equals", "gte", "lte"] : field === "captureDate" || field === "importDate" ? ["before", "after", "between"] : ["fileType", "keyword", "camera", "lens"].includes(field) ? ["is", "contains", "notContains"] : ["is"];
const initialRule = (): SmartRule => ({ field: "rating", operator: "gte", value: 4 });

export function CatalogueOrganiser({ filter, onFilter, selectedIds, active, onChanged, onNotice }: Props) {
  const [collections, setCollections] = useState<CatalogueCollection[]>([]);
  const [sets, setSets] = useState<CollectionSet[]>([]);
  const [versions, setVersions] = useState<CatalogueVersion[]>([]);
  const [editingSmart, setEditingSmart] = useState(false);
  const [smartName, setSmartName] = useState("Four stars or better");
  const [matchMode, setMatchMode] = useState<"all" | "any">("all");
  const [rules, setRules] = useState<SmartRule[]>([initialRule()]);
  const [nameAction, setNameAction] = useState<NameAction | null>(null);
  const [nameValue, setNameValue] = useState("");

  const refresh = async () => { setCollections(await api.collections()); setSets(await api.collectionSets()); setVersions(active ? await api.catalogueVersions(active.id) : []); };
  useEffect(() => { void refresh(); }, [active?.id]);
  const changed = async (message: string) => { await refresh(); await onChanged(); onNotice(message); };
  const openName = (action: NameAction, value = "") => { setNameAction(action); setNameValue(value); };
  const saveSmart = async () => { await api.saveCollection({ name: smartName, kind: "smart", matchMode, rules }); setEditingSmart(false); await changed(`Smart Collection “${smartName}” saved.`); };
  const updateRule = (index: number, patch: Partial<SmartRule>) => setRules(value => value.map((rule, position) => position === index ? { ...rule, ...patch } : rule));
  const saveName = async () => {
    if (!nameAction) return;
    const name = nameValue.trim();
    if (nameAction.kind === "createCollection") {
      if (!name) return;
      await api.saveCollection({ name, kind: "manual", setId: nameAction.setId, matchMode: "all", rules: [] });
      await changed(`Collection “${name}” created.`);
    } else if (nameAction.kind === "createSet") {
      if (!name) return;
      await api.createCollectionSet(name);
      await changed(`Collection Set “${name}” created.`);
    } else if (nameAction.kind === "renameCollection") {
      if (!name) return;
      await api.saveCollection({ ...nameAction.collection, name });
      await changed(`Collection renamed to “${name}”.`);
    } else if (nameAction.kind === "renameSet") {
      if (!name) return;
      await api.renameCollectionSet(nameAction.set.id, name);
      await changed(`Collection Set renamed to “${name}”.`);
    } else if (nameAction.kind === "renameVersion") {
      if (!name) return;
      await api.renameCatalogueVersion(nameAction.version.id, name);
      await changed("Version renamed.");
    } else if (active) {
      const created = await api.createCatalogueVersion(active.id, nameAction.mode, name || undefined);
      onFilter({ search: created.versionName ?? "" });
      window.dispatchEvent(new CustomEvent("keepframe-select-catalogue-item", { detail: { id: created.id } }));
      await changed(`${created.versionName ?? "Version"} created without duplicating the source file.`);
    }
    setNameAction(null);
  };
  const collectionRow = (collection: CatalogueCollection) => <CollectionRow key={collection.id} collection={collection} active={filter.collectionId === collection.id} selectedCount={selectedIds.size} onOpen={() => onFilter({ collectionId: collection.id, search: "" })} onRename={() => openName({ kind: "renameCollection", collection }, collection.name)} onAdd={() => void api.updateCollectionMembers(collection.id, [...selectedIds], true).then(() => changed(`${selectedIds.size} selected item(s) added to “${collection.name}”.`))} onRemove={() => void api.updateCollectionMembers(collection.id, [...selectedIds], false).then(() => changed(`Selected item(s) removed from “${collection.name}”.`))} onDelete={() => void api.deleteCollection(collection.id).then(() => { if (filter.collectionId === collection.id) onFilter({ collectionId: undefined }); return changed(`Collection “${collection.name}” deleted; photographs were retained.`); })} />;

  return <aside className="catalogue-organiser" aria-label="Catalogue organisation">
    <section>
      <header><h2>Collections</h2><span><button title="New Collection" aria-label="New Collection" onClick={() => openName({ kind: "createCollection" }, "New Collection")}><Plus size={14} /></button><button title="New Collection Set" aria-label="New Collection Set" onClick={() => openName({ kind: "createSet" }, "New Set")}><FolderPlus size={14} /></button></span></header>
      <button className={!filter.collectionId ? "organiser-row active" : "organiser-row"} onClick={() => onFilter({ collectionId: undefined })}><span>All Photographs</span></button>
      {sets.map(set => <div key={set.id} className="collection-set"><div className="collection-set-heading"><strong><ChevronDown size={13} />{set.name}</strong><button title={`New Collection in ${set.name}`} aria-label={`New Collection in ${set.name}`} onClick={() => openName({ kind: "createCollection", setId: set.id }, "New Collection")}>+</button><button aria-label={`Rename ${set.name}`} onClick={() => openName({ kind: "renameSet", set }, set.name)}>✎</button><button aria-label={`Delete ${set.name}`} onClick={() => window.confirm(`Delete Collection Set “${set.name}”? Its Collections will move to the root; no photographs will be deleted.`) && void api.deleteCollectionSet(set.id).then(() => changed(`Collection Set “${set.name}” deleted; Collections and photographs retained.`))}><Trash2 size={12} /></button></div>{collections.filter(value => value.setId === set.id).map(collectionRow)}</div>)}
      {collections.filter(value => !value.setId).map(collectionRow)}
      <button className="organiser-create" onClick={() => setEditingSmart(true)}>+ Smart Collection</button>
    </section>
    <section>
      <header><h2>Versions</h2></header>{active ? <><p className="organiser-caption">{active.sourceVersionCount ?? (versions.length || 1)} item(s), one protected source</p><div className="version-actions"><button onClick={() => openName({ kind: "createVersion", mode: "current" })}>From current</button><button onClick={() => openName({ kind: "createVersion", mode: "default" })}>From original</button><button onClick={() => openName({ kind: "createVersion", mode: "duplicate" })}><CopyPlus size={13} /> Duplicate</button></div>
        {versions.map(version => <div className="organiser-row" key={version.id}><button title="Show this version" onClick={() => { onFilter({ search: version.isPrimary ? active.filename : version.name }); window.dispatchEvent(new CustomEvent("keepframe-select-catalogue-item", { detail: { id: version.id } })); }}><span>{version.isPrimary ? "Primary" : version.name}</span><small>{version.hasEdits ? "Edited" : "As shot"}</small></button>{!version.isPrimary ? <><button aria-label={`Rename ${version.name}`} onClick={() => openName({ kind: "renameVersion", version }, version.name)}>✎</button><button aria-label={`Delete ${version.name}`} onClick={() => window.confirm(`Delete “${version.name}”? The protected source file will not be touched.`) && void api.deleteCatalogueVersion(version.id).then(() => changed("Virtual version deleted; source retained."))}><Trash2 size={13} /></button></> : null}</div>)}
        {versions.length > 1 ? <button className="organiser-create" onClick={() => void api.setVersionGroupCollapsed(active.id, !active.versionGroupCollapsed).then(() => changed(active.versionGroupCollapsed ? "Sibling versions expanded." : "Sibling versions collapsed to the primary item."))}>{active.versionGroupCollapsed ? <ChevronRight size={13} /> : <ChevronDown size={13} />} {active.versionGroupCollapsed ? "Expand" : "Collapse"} siblings</button> : null}</> : <p className="organiser-caption">Select an item to manage its versions.</p>}
    </section>
    <section>
      <header><h2>Stacks</h2></header><button disabled={selectedIds.size < 2 || !active || Boolean(active.stackId)} onClick={() => active && void api.createStack([...selectedIds], active.id).then(() => changed("Stack created. Normal commands still target only selected visible items."))}><Layers3 size={14} /> Stack selection</button>
      {active?.stackId ? <div className="version-actions"><button disabled={selectedIds.size < 2} onClick={() => void api.addToStack(active.stackId!, [...selectedIds]).then(count => changed(`${count} item(s) added to the active stack.`))}>Add selection</button><button onClick={() => void api.removeFromStack(active.id).then(() => changed("Active item removed from the stack; source retained."))}>Remove active</button><button onClick={() => void api.setStackCollapsed(active.stackId!, !active.stackCollapsed).then(() => changed(active.stackCollapsed ? "Stack expanded." : "Stack collapsed to its top item."))}>{active.stackCollapsed ? "Expand" : "Collapse"}</button><button disabled={active.isStackTop} onClick={() => void api.setStackTop(active.stackId!, active.id).then(() => changed("Stack Top changed."))}>Set as Top</button><button onClick={() => window.confirm("Unstack these items? Photographs and versions will remain.") && void api.unstack(active.stackId!).then(() => changed("Stack metadata removed; photographs retained."))}>Unstack</button></div> : null}
    </section>
    {editingSmart ? <div className="modal-backdrop"><section className="modal smart-editor" role="dialog" aria-modal="true" aria-label="Smart Collection editor"><button className="modal-close" onClick={() => setEditingSmart(false)}>×</button><h2>Smart Collection</h2><label>Name<input value={smartName} onChange={event => setSmartName(event.target.value)} /></label><label>Rules<select value={matchMode} onChange={event => setMatchMode(event.target.value as "all" | "any")}><option value="all">Match ALL</option><option value="any">Match ANY</option></select></label>
      {rules.map((rule, index) => <div className="smart-rule" key={index}><select aria-label={`Rule ${index + 1} field`} value={rule.field} onChange={event => { const field = event.target.value as SmartRule["field"]; updateRule(index, { field, operator: operators(field)[0], value: field === "rating" ? 4 : field === "edited" || field === "hasMultipleVersions" ? true : "" }); }}>{fields.map(field => <option key={field} value={field}>{field}</option>)}</select><select aria-label={`Rule ${index + 1} operator`} value={rule.operator} onChange={event => updateRule(index, { operator: event.target.value })}>{operators(rule.field).map(operator => <option key={operator}>{operator}</option>)}</select>{rule.field === "edited" || rule.field === "hasMultipleVersions" ? <select aria-label={`Rule ${index + 1} value`} value={String(rule.value)} onChange={event => updateRule(index, { value: event.target.value === "true" })}><option value="true">True</option><option value="false">False</option></select> : <input aria-label={`Rule ${index + 1} value`} type={rule.field === "rating" ? "number" : "text"} min={0} max={5} value={String(rule.value)} onChange={event => updateRule(index, { value: rule.field === "rating" ? Number(event.target.value) : event.target.value })} />}{rule.operator === "between" ? <input aria-label={`Rule ${index + 1} second value`} value={String(rule.secondValue ?? "")} onChange={event => updateRule(index, { secondValue: event.target.value })} /> : null}<button aria-label={`Remove rule ${index + 1}`} disabled={rules.length === 1} onClick={() => setRules(value => value.filter((_, position) => position !== index))}>×</button></div>)}
      <button onClick={() => setRules(value => [...value, initialRule()])}>+ Add rule</button><div className="modal-actions"><button onClick={() => setEditingSmart(false)}>Cancel</button><button className="primary-button" disabled={!smartName.trim()} onClick={() => void saveSmart()}>Save Smart Collection</button></div></section></div> : null}
    {nameAction ? <div className="modal-backdrop"><section className="modal name-editor" role="dialog" aria-modal="true" aria-label={nameAction.kind.includes("Version") ? "Version name" : nameAction.kind.includes("Set") ? "Collection Set name" : "Collection name"}><button className="modal-close" aria-label="Close name editor" onClick={() => setNameAction(null)}>×</button><h2>{nameAction.kind.startsWith("rename") ? "Rename" : "Create"} {nameAction.kind.includes("Version") ? "Version" : nameAction.kind.includes("Set") ? "Collection Set" : "Collection"}</h2><label>Name<input autoFocus value={nameValue} placeholder={nameAction.kind === "createVersion" ? "Optional — a safe default will be generated" : undefined} onChange={event => setNameValue(event.target.value)} onKeyDown={event => { if (event.key === "Enter") void saveName(); }} /></label><div className="modal-actions"><button onClick={() => setNameAction(null)}>Cancel</button><button className="primary-button" disabled={nameAction.kind !== "createVersion" && !nameValue.trim()} onClick={() => void saveName()}>{nameAction.kind.startsWith("rename") ? "Rename" : "Create"}</button></div></section></div> : null}
  </aside>;
}

function CollectionRow({ collection, active, selectedCount, onOpen, onRename, onAdd, onRemove, onDelete }: { collection: CatalogueCollection; active: boolean; selectedCount: number; onOpen: () => void; onRename: () => void; onAdd: () => void; onRemove: () => void; onDelete: () => void }) {
  return <div className={active ? "organiser-row active" : "organiser-row"}><button onClick={onOpen}><span>{collection.kind === "smart" ? "◆ " : ""}{collection.name}</span><small>{collection.count}</small></button><button title="Rename collection" aria-label={`Rename ${collection.name}`} onClick={onRename}>✎</button>{collection.kind === "manual" ? <><button title="Add selected" disabled={!selectedCount} onClick={onAdd}>+</button><button title="Remove selected" disabled={!selectedCount} onClick={onRemove}>−</button></> : null}<button title="Delete collection" aria-label={`Delete ${collection.name}`} onClick={() => window.confirm(`Delete “${collection.name}”? No photographs or versions will be deleted.`) && onDelete()}><Trash2 size={12} /></button></div>;
}
