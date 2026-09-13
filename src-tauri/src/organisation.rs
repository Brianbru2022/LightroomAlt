use super::{KeepframeError, Result};
use chrono::Utc;
use rusqlite::{params, params_from_iter, types::Value as SqlValue, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use uuid::Uuid;

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CatalogueVersion {
    pub(crate) id: String,
    pub(crate) source_id: String,
    pub(crate) name: String,
    pub(crate) is_primary: bool,
    pub(crate) version_index: i64,
    pub(crate) has_edits: bool,
    pub(crate) rating: i64,
    pub(crate) decision: String,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateVersionRequest {
    pub(crate) item_id: String,
    pub(crate) mode: String,
    pub(crate) name: Option<String>,
}
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SmartRule {
    pub(crate) field: String,
    pub(crate) operator: String,
    pub(crate) value: Value,
    pub(crate) second_value: Option<Value>,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SaveCollectionRequest {
    pub(crate) id: Option<String>,
    pub(crate) name: String,
    pub(crate) kind: String,
    pub(crate) set_id: Option<String>,
    pub(crate) match_mode: String,
    pub(crate) rules: Vec<SmartRule>,
}
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Collection {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) kind: String,
    pub(crate) set_id: Option<String>,
    pub(crate) match_mode: String,
    pub(crate) rules: Vec<SmartRule>,
    pub(crate) position: i64,
    pub(crate) count: i64,
}
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CollectionSet {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) position: i64,
}
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StackSummary {
    pub(crate) id: String,
    pub(crate) name: Option<String>,
    pub(crate) collapsed: bool,
    pub(crate) top_item_id: String,
    pub(crate) member_ids: Vec<String>,
}

fn clean_name(value: &str, noun: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > 120 || value.chars().any(|ch| ch == '\0') {
        return Err(KeepframeError::Message(format!(
            "{noun} name must be 1 to 120 characters."
        )));
    }
    Ok(value.into())
}
fn bool_value(value: &Value) -> Result<bool> {
    value.as_bool().ok_or_else(|| {
        KeepframeError::Message("Smart Collection rule requires true or false.".into())
    })
}
fn text_value(value: &Value) -> Result<String> {
    let text = value
        .as_str()
        .ok_or_else(|| KeepframeError::Message("Smart Collection rule requires text.".into()))?
        .trim();
    if text.is_empty() || text.chars().count() > 512 || text.chars().any(|ch| ch == '\0') {
        return Err(KeepframeError::Message(
            "Smart Collection text must be 1 to 512 characters.".into(),
        ));
    }
    Ok(text.into())
}
fn like_value(value: &str) -> String {
    format!(
        "%{}%",
        value
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    )
}

pub(crate) fn compile_rules(
    rules: &[SmartRule],
    match_mode: &str,
) -> Result<(String, Vec<SqlValue>)> {
    if !matches!(match_mode, "all" | "any") || rules.is_empty() || rules.len() > 20 {
        return Err(KeepframeError::Message(
            "Smart Collections require 1 to 20 rules and Match ALL or Match ANY.".into(),
        ));
    }
    let mut clauses = Vec::new();
    let mut values = Vec::new();
    for rule in rules {
        match rule.field.as_str() {
            "rating" => {
                let number = rule
                    .value
                    .as_i64()
                    .filter(|value| (0..=5).contains(value))
                    .ok_or_else(|| {
                        KeepframeError::Message("Rating rules require 0 to 5.".into())
                    })?;
                let operator = match rule.operator.as_str() {
                    "equals" => "=",
                    "gte" => ">=",
                    "lte" => "<=",
                    _ => {
                        return Err(KeepframeError::Message(
                            "Unsupported rating operator.".into(),
                        ))
                    }
                };
                clauses.push(format!("a.rating {operator} ?"));
                values.push(SqlValue::Integer(number));
            }
            "flag" => {
                if rule.operator != "is" {
                    return Err(KeepframeError::Message(
                        "Flag rules support only ‘is’.".into(),
                    ));
                }
                let value = text_value(&rule.value)?;
                if !matches!(value.as_str(), "keep" | "undecided" | "discard") {
                    return Err(KeepframeError::Message(
                        "Flag must be Pick, Unflagged or Reject.".into(),
                    ));
                }
                clauses.push("a.decision=?".into());
                values.push(SqlValue::Text(value));
            }
            "edited" => {
                if rule.operator != "is" {
                    return Err(KeepframeError::Message(
                        "Edited rules support only ‘is’.".into(),
                    ));
                }
                clauses.push(
                    if bool_value(&rule.value)? {
                        "EXISTS(SELECT 1 FROM develop_recipes scr WHERE scr.asset_id=a.id)"
                    } else {
                        "NOT EXISTS(SELECT 1 FROM develop_recipes scr WHERE scr.asset_id=a.id)"
                    }
                    .into(),
                );
            }
            "fileType" => {
                let text = text_value(&rule.value)?
                    .trim_start_matches('.')
                    .to_lowercase();
                match rule.operator.as_str() {
                    "is" => {
                        clauses.push("EXISTS(SELECT 1 FROM representations sr WHERE sr.source_id=a.source_id AND lower(sr.extension)=?)".into());
                        values.push(SqlValue::Text(text));
                    }
                    "contains" => {
                        clauses.push("EXISTS(SELECT 1 FROM representations sr WHERE sr.source_id=a.source_id AND lower(sr.extension) LIKE ? ESCAPE '\\')".into());
                        values.push(SqlValue::Text(like_value(&text)));
                    }
                    "notContains" => {
                        clauses.push("NOT EXISTS(SELECT 1 FROM representations sr WHERE sr.source_id=a.source_id AND lower(sr.extension) LIKE ? ESCAPE '\\')".into());
                        values.push(SqlValue::Text(like_value(&text)));
                    }
                    _ => {
                        return Err(KeepframeError::Message(
                            "Unsupported file-type operator.".into(),
                        ))
                    }
                }
            }
            "keyword" => {
                let text = text_value(&rule.value)?.to_lowercase();
                let exists="SELECT 1 FROM asset_tags sk JOIN tags st ON st.id=sk.tag_id WHERE sk.asset_id=a.id AND lower(st.name)";
                match rule.operator.as_str() {
                    "is" => {
                        clauses.push(format!("EXISTS({exists}=?)"));
                        values.push(SqlValue::Text(text));
                    }
                    "contains" => {
                        clauses.push(format!("EXISTS({exists} LIKE ? ESCAPE '\\')"));
                        values.push(SqlValue::Text(like_value(&text)));
                    }
                    "notContains" => {
                        clauses.push(format!("NOT EXISTS({exists} LIKE ? ESCAPE '\\')"));
                        values.push(SqlValue::Text(like_value(&text)));
                    }
                    _ => {
                        return Err(KeepframeError::Message(
                            "Unsupported keyword operator.".into(),
                        ))
                    }
                }
            }
            "captureDate" | "importDate" => {
                let column = if rule.field == "captureDate" {
                    "s.captured_at"
                } else {
                    "a.created_at"
                };
                let first = text_value(&rule.value)?;
                match rule.operator.as_str() {
                    "before" => {
                        clauses.push(format!("{column} < ?"));
                        values.push(SqlValue::Text(first));
                    }
                    "after" => {
                        clauses.push(format!("{column} > ?"));
                        values.push(SqlValue::Text(first));
                    }
                    "between" => {
                        let second = text_value(rule.second_value.as_ref().ok_or_else(|| {
                            KeepframeError::Message("Between requires two dates.".into())
                        })?)?;
                        clauses.push(format!("{column} BETWEEN ? AND ?"));
                        values.push(SqlValue::Text(first));
                        values.push(SqlValue::Text(second));
                    }
                    _ => return Err(KeepframeError::Message("Unsupported date operator.".into())),
                }
            }
            "camera" | "lens" => {
                let column = if rule.field == "camera" {
                    "COALESCE(s.camera,'')"
                } else {
                    "COALESCE(s.lens,'')"
                };
                let text = text_value(&rule.value)?.to_lowercase();
                match rule.operator.as_str() {
                    "is" => {
                        clauses.push(format!("lower({column})=?"));
                        values.push(SqlValue::Text(text));
                    }
                    "contains" => {
                        clauses.push(format!("lower({column}) LIKE ? ESCAPE '\\'"));
                        values.push(SqlValue::Text(like_value(&text)));
                    }
                    "notContains" => {
                        clauses.push(format!("lower({column}) NOT LIKE ? ESCAPE '\\'"));
                        values.push(SqlValue::Text(like_value(&text)));
                    }
                    _ => {
                        return Err(KeepframeError::Message(
                            "Unsupported camera/lens operator.".into(),
                        ))
                    }
                }
            }
            "versionStatus" => {
                if rule.operator != "is" {
                    return Err(KeepframeError::Message(
                        "Version status supports only ‘is’.".into(),
                    ));
                }
                clauses.push(
                    match text_value(&rule.value)?.as_str() {
                        "primary" => "a.is_primary=1",
                        "virtual" => "a.is_primary=0",
                        _ => {
                            return Err(KeepframeError::Message(
                                "Version status must be primary or virtual.".into(),
                            ))
                        }
                    }
                    .into(),
                );
            }
            "hasMultipleVersions" => {
                if rule.operator != "is" {
                    return Err(KeepframeError::Message(
                        "Version-count rules support only ‘is’.".into(),
                    ));
                }
                let comparison = if bool_value(&rule.value)? {
                    "> 1"
                } else {
                    "= 1"
                };
                clauses.push(format!("(SELECT count(*) FROM assets av WHERE av.source_id=a.source_id AND av.trashed_at IS NULL) {comparison}"));
            }
            "stackStatus" => {
                if rule.operator != "is" {
                    return Err(KeepframeError::Message(
                        "Stack status supports only ‘is’.".into(),
                    ));
                }
                clauses.push(
                    match text_value(&rule.value)?.as_str() {
                        "stacked" => "EXISTS(SELECT 1 FROM stack_items sx WHERE sx.item_id=a.id)",
                        "unstacked" => {
                            "NOT EXISTS(SELECT 1 FROM stack_items sx WHERE sx.item_id=a.id)"
                        }
                        _ => {
                            return Err(KeepframeError::Message(
                                "Stack status must be stacked or unstacked.".into(),
                            ))
                        }
                    }
                    .into(),
                );
            }
            _ => {
                return Err(KeepframeError::Message(format!(
                    "Unsupported Smart Collection field: {}",
                    rule.field
                )))
            }
        }
    }
    Ok((
        format!(
            "({})",
            clauses.join(if match_mode == "all" { " AND " } else { " OR " })
        ),
        values,
    ))
}

pub(crate) fn collection_filter_clause(
    connection: &Connection,
    id: &str,
) -> Result<(String, Vec<SqlValue>)> {
    let record: Option<(String, String, String)> = connection
        .query_row(
            "SELECT kind,match_mode,rules_json FROM collections WHERE id=?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let Some((kind, mode, json)) = record else {
        return Err(KeepframeError::Message(
            "That Collection no longer exists.".into(),
        ));
    };
    if kind == "manual" {
        return Ok(("EXISTS(SELECT 1 FROM collection_items ci WHERE ci.collection_id=? AND ci.item_id=a.id)".into(),vec![SqlValue::Text(id.into())]));
    }
    let rules = serde_json::from_str::<Vec<SmartRule>>(&json)
        .map_err(|_| KeepframeError::Message("Smart Collection contains invalid rules.".into()))?;
    compile_rules(&rules, &mode)
}

pub(crate) fn list_versions_in(
    connection: &Connection,
    item_id: &str,
) -> Result<Vec<CatalogueVersion>> {
    let source_id: String = connection.query_row(
        "SELECT source_id FROM assets WHERE id=?1",
        [item_id],
        |row| row.get(0),
    )?;
    let mut statement=connection.prepare("SELECT a.id,a.source_id,COALESCE(a.version_name,CASE WHEN a.is_primary=1 THEN 'Primary' ELSE 'Version '||a.version_index END),a.is_primary,a.version_index,EXISTS(SELECT 1 FROM develop_recipes dr WHERE dr.asset_id=a.id),a.rating,a.decision FROM assets a WHERE a.source_id=?1 AND a.trashed_at IS NULL ORDER BY a.version_index,a.id")?;
    let rows = statement
        .query_map([source_id], |row| {
            Ok(CatalogueVersion {
                id: row.get(0)?,
                source_id: row.get(1)?,
                name: row.get(2)?,
                is_primary: row.get::<_, i64>(3)? != 0,
                version_index: row.get(4)?,
                has_edits: row.get::<_, i64>(5)? != 0,
                rating: row.get(6)?,
                decision: row.get(7)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub(crate) fn create_version_in(
    connection: &mut Connection,
    request: CreateVersionRequest,
) -> Result<String> {
    if !matches!(request.mode.as_str(), "current" | "default" | "duplicate") {
        return Err(KeepframeError::Message(
            "Version mode must be current, default or duplicate.".into(),
        ));
    }
    let(source_id,index):(String,i64)=connection.query_row("SELECT source_id,(SELECT COALESCE(max(version_index),0)+1 FROM assets sibling WHERE sibling.source_id=a.source_id) FROM assets a WHERE a.id=?1 AND a.trashed_at IS NULL",[&request.item_id],|row|Ok((row.get(0)?,row.get(1)?)))?;
    let id = Uuid::new_v4().to_string();
    let name = match request.name {
        Some(value) => clean_name(&value, "Version")?,
        None => format!("Version {index}"),
    };
    let now = Utc::now().to_rfc3339();
    let copy_metadata = request.mode == "duplicate";
    let copy_recipe = request.mode != "default";
    let tx = connection.transaction()?;
    tx.execute("INSERT INTO assets(id,filename,decision,rating,title,caption,copyright,creator,captured_at,date_fallback,camera,width,height,latitude,longitude,embedded_latitude,embedded_longitude,manual_latitude,manual_longitude,location_source,missing_state,last_verified_at,thumbnail_path,preferred_version_id,created_at,trashed_at,source_id,is_primary,version_name,version_index) SELECT ?1,filename,CASE WHEN ?5 THEN decision ELSE 'undecided' END,CASE WHEN ?5 THEN rating ELSE 0 END,CASE WHEN ?5 THEN title ELSE NULL END,CASE WHEN ?5 THEN caption ELSE NULL END,copyright,creator,captured_at,date_fallback,camera,width,height,latitude,longitude,embedded_latitude,embedded_longitude,manual_latitude,manual_longitude,location_source,missing_state,last_verified_at,thumbnail_path,CASE WHEN ?6 THEN preferred_version_id ELSE NULL END,?4,NULL,?2,0,?3,?7 FROM assets WHERE id=?8",params![id,source_id,name,now,copy_metadata,copy_recipe,index,request.item_id])?;
    if copy_recipe {
        tx.execute("INSERT INTO develop_recipes(asset_id,schema_version,recipe_json,updated_at) SELECT ?1,schema_version,recipe_json,?2 FROM develop_recipes WHERE asset_id=?3",params![id,now,request.item_id])?;
    }
    if copy_metadata {
        tx.execute("INSERT INTO asset_tags(asset_id,tag_id) SELECT ?1,tag_id FROM asset_tags WHERE asset_id=?2",params![id,request.item_id])?;
    }
    tx.commit()?;
    Ok(id)
}
pub(crate) fn rename_version_in(connection: &Connection, item_id: &str, name: &str) -> Result<()> {
    let name = clean_name(name, "Version")?;
    if connection.execute(
        "UPDATE assets SET version_name=?2 WHERE id=?1 AND is_primary=0",
        params![item_id, name],
    )? != 1
    {
        return Err(KeepframeError::Message(
            "Only virtual versions can be renamed with this action.".into(),
        ));
    }
    Ok(())
}
pub(crate) fn delete_version_in(connection: &Connection, item_id: &str) -> Result<String> {
    let (source_id, is_primary): (String, i64) = connection.query_row(
        "SELECT source_id,is_primary FROM assets WHERE id=?1",
        [item_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if is_primary != 0 {
        return Err(KeepframeError::Message(
            "The primary item cannot be deleted as a version. Use the source-removal workflow."
                .into(),
        ));
    }
    let top: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM stacks WHERE top_item_id=?1)",
        [item_id],
        |row| row.get(0),
    )?;
    if top {
        return Err(KeepframeError::Message(
            "Choose another Stack Top or unstack before deleting this version.".into(),
        ));
    }
    connection.execute("DELETE FROM assets WHERE id=?1", [item_id])?;
    Ok(source_id)
}
pub(crate) fn set_versions_collapsed_in(
    connection: &Connection,
    item_id: &str,
    collapsed: bool,
) -> Result<()> {
    if connection.execute("UPDATE sources SET versions_collapsed=?2 WHERE id=(SELECT source_id FROM assets WHERE id=?1)",params![item_id,collapsed])?!=1{return Err(KeepframeError::Message("That source no longer exists.".into()));}
    Ok(())
}

pub(crate) fn save_collection_in(
    connection: &mut Connection,
    request: SaveCollectionRequest,
) -> Result<String> {
    let name = clean_name(&request.name, "Collection")?;
    if !matches!(request.kind.as_str(), "manual" | "smart") {
        return Err(KeepframeError::Message(
            "Collection kind must be manual or smart.".into(),
        ));
    }
    if request.kind == "smart" {
        compile_rules(&request.rules, &request.match_mode)?;
    } else if !request.rules.is_empty() {
        return Err(KeepframeError::Message(
            "Manual Collections cannot contain Smart rules.".into(),
        ));
    }
    let id = request.id.unwrap_or_else(|| Uuid::new_v4().to_string());
    let now = Utc::now().to_rfc3339();
    let rules = serde_json::to_string(&request.rules)?;
    let tx = connection.transaction()?;
    let position: i64 = tx.query_row(
        "SELECT COALESCE(max(position),-1)+1 FROM collections",
        [],
        |row| row.get(0),
    )?;
    tx.execute("INSERT INTO collections(id,name,kind,set_id,match_mode,rules_json,position,created_at,updated_at)VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?8) ON CONFLICT(id) DO UPDATE SET name=excluded.name,kind=excluded.kind,set_id=excluded.set_id,match_mode=excluded.match_mode,rules_json=excluded.rules_json,updated_at=excluded.updated_at",params![id,name,request.kind,request.set_id,request.match_mode,rules,position,now])?;
    tx.commit()?;
    Ok(id)
}
pub(crate) fn delete_collection_in(connection: &Connection, id: &str) -> Result<()> {
    if connection.execute("DELETE FROM collections WHERE id=?1", [id])? != 1 {
        return Err(KeepframeError::Message(
            "That Collection no longer exists.".into(),
        ));
    }
    Ok(())
}
pub(crate) fn update_collection_members_in(
    connection: &mut Connection,
    collection_id: &str,
    item_ids: Vec<String>,
    add: bool,
) -> Result<usize> {
    let kind: String = connection.query_row(
        "SELECT kind FROM collections WHERE id=?1",
        [collection_id],
        |row| row.get(0),
    )?;
    if kind != "manual" {
        return Err(KeepframeError::Message(
            "Smart Collection membership is evaluated from rules.".into(),
        ));
    }
    let mut seen = HashSet::new();
    let ids = item_ids
        .into_iter()
        .filter(|item| seen.insert(item.clone()))
        .collect::<Vec<_>>();
    let tx = connection.transaction()?;
    let first_position: i64 = tx.query_row(
        "SELECT COALESCE(max(position),-1)+1 FROM collection_items WHERE collection_id=?1",
        [collection_id],
        |row| row.get(0),
    )?;
    let mut changed = 0;
    for (offset, id) in ids.into_iter().enumerate() {
        if add {
            let position = first_position + offset as i64;
            changed+=tx.execute("INSERT OR IGNORE INTO collection_items(collection_id,item_id,position,added_at) SELECT ?1,id,?3,?4 FROM assets WHERE id=?2 AND trashed_at IS NULL",params![collection_id,id,position,Utc::now().to_rfc3339()])?;
        } else {
            changed += tx.execute(
                "DELETE FROM collection_items WHERE collection_id=?1 AND item_id=?2",
                params![collection_id, id],
            )?;
        }
    }
    tx.commit()?;
    Ok(changed)
}
pub(crate) fn list_collections_in(connection: &Connection) -> Result<Vec<Collection>> {
    let mut statement=connection.prepare("SELECT id,name,kind,set_id,match_mode,rules_json,position FROM collections ORDER BY position,name COLLATE NOCASE,id")?;
    let records = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, i64>(6)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    records.into_iter().map(|(id,name,kind,set_id,match_mode,json,position)|{let rules=serde_json::from_str::<Vec<SmartRule>>(&json).map_err(|_|KeepframeError::Message(format!("Smart Collection ‘{name}’ has invalid rules.")))?;let count=if kind=="manual"{connection.query_row("SELECT count(*) FROM collection_items ci JOIN assets a ON a.id=ci.item_id WHERE ci.collection_id=?1 AND a.trashed_at IS NULL",[&id],|row|row.get(0))?}else{let(clause,values)=compile_rules(&rules,&match_mode)?;connection.query_row(&format!("SELECT count(*) FROM assets a JOIN sources s ON s.id=a.source_id WHERE a.trashed_at IS NULL AND {clause}"),params_from_iter(values.iter()),|row|row.get(0))?};Ok(Collection{id,name,kind,set_id,match_mode,rules,position,count})}).collect()
}

pub(crate) fn list_collection_sets_in(connection: &Connection) -> Result<Vec<CollectionSet>> {
    let mut statement = connection.prepare(
        "SELECT id,name,position FROM collection_sets ORDER BY position,name COLLATE NOCASE,id",
    )?;
    let rows = statement
        .query_map([], |row| {
            Ok(CollectionSet {
                id: row.get(0)?,
                name: row.get(1)?,
                position: row.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}
pub(crate) fn create_collection_set_in(connection: &Connection, name: &str) -> Result<String> {
    let name = clean_name(name, "Collection Set")?;
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    let position: i64 = connection.query_row(
        "SELECT COALESCE(max(position),-1)+1 FROM collection_sets",
        [],
        |row| row.get(0),
    )?;
    connection.execute(
        "INSERT INTO collection_sets(id,name,position,created_at,updated_at)VALUES(?1,?2,?3,?4,?4)",
        params![id, name, position, now],
    )?;
    Ok(id)
}
pub(crate) fn rename_collection_set_in(
    connection: &Connection,
    id: &str,
    name: &str,
) -> Result<()> {
    let name = clean_name(name, "Collection Set")?;
    if connection.execute(
        "UPDATE collection_sets SET name=?2,updated_at=?3 WHERE id=?1",
        params![id, name, Utc::now().to_rfc3339()],
    )? != 1
    {
        return Err(KeepframeError::Message(
            "That Collection Set no longer exists.".into(),
        ));
    }
    Ok(())
}
pub(crate) fn delete_collection_set_in(connection: &Connection, id: &str) -> Result<()> {
    if connection.execute("DELETE FROM collection_sets WHERE id=?1", [id])? != 1 {
        return Err(KeepframeError::Message(
            "That Collection Set no longer exists.".into(),
        ));
    }
    Ok(())
}

pub(crate) fn create_stack_in(
    connection: &mut Connection,
    item_ids: Vec<String>,
    top_item_id: &str,
) -> Result<String> {
    let ids = item_ids.into_iter().collect::<HashSet<_>>();
    if ids.len() < 2 || !ids.contains(top_item_id) {
        return Err(KeepframeError::Message(
            "Select at least two items and an active Stack Top.".into(),
        ));
    }
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    let tx = connection.transaction()?;
    let valid: i64 = tx.query_row(
        &format!(
            "SELECT count(*) FROM assets WHERE trashed_at IS NULL AND id IN ({})",
            std::iter::repeat_n("?", ids.len())
                .collect::<Vec<_>>()
                .join(",")
        ),
        params_from_iter(ids.iter()),
        |row| row.get(0),
    )?;
    if valid != ids.len() as i64 {
        return Err(KeepframeError::Message(
            "A selected Stack item no longer exists.".into(),
        ));
    }
    for item in &ids {
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM stack_items WHERE item_id=?1)",
            [item],
            |row| row.get(0),
        )?;
        if exists {
            return Err(KeepframeError::Message(
                "An item can belong to only one manual Stack.".into(),
            ));
        }
    }
    tx.execute(
        "INSERT INTO stacks(id,collapsed,top_item_id,created_at,updated_at)VALUES(?1,1,?2,?3,?3)",
        params![id, top_item_id, now],
    )?;
    let mut ordered = ids.into_iter().collect::<Vec<_>>();
    ordered.sort();
    ordered.retain(|item| item != top_item_id);
    ordered.insert(0, top_item_id.into());
    for (item, position) in ordered.iter().zip(0_i64..) {
        tx.execute(
            "INSERT INTO stack_items(stack_id,item_id,position)VALUES(?1,?2,?3)",
            params![id, item, position],
        )?;
    }
    tx.commit()?;
    Ok(id)
}
pub(crate) fn list_stacks_in(connection: &Connection) -> Result<Vec<StackSummary>> {
    let mut statement = connection
        .prepare("SELECT id,name,collapsed,top_item_id FROM stacks ORDER BY created_at,id")?;
    let records = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, i64>(2)? != 0,
                row.get::<_, String>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    records
        .into_iter()
        .map(|(id, name, collapsed, top)| {
            let mut members = connection.prepare(
                "SELECT item_id FROM stack_items WHERE stack_id=?1 ORDER BY position,item_id",
            )?;
            let member_ids = members
                .query_map([&id], |row| row.get(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(StackSummary {
                id,
                name,
                collapsed,
                top_item_id: top,
                member_ids,
            })
        })
        .collect()
}
pub(crate) fn set_stack_collapsed_in(
    connection: &Connection,
    id: &str,
    collapsed: bool,
) -> Result<()> {
    if connection.execute(
        "UPDATE stacks SET collapsed=?2,updated_at=?3 WHERE id=?1",
        params![id, collapsed, Utc::now().to_rfc3339()],
    )? != 1
    {
        return Err(KeepframeError::Message(
            "That Stack no longer exists.".into(),
        ));
    }
    Ok(())
}
pub(crate) fn set_stack_top_in(connection: &Connection, id: &str, item_id: &str) -> Result<()> {
    let belongs: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM stack_items WHERE stack_id=?1 AND item_id=?2)",
        params![id, item_id],
        |row| row.get(0),
    )?;
    if !belongs {
        return Err(KeepframeError::Message(
            "Stack Top must be a member of that Stack.".into(),
        ));
    }
    connection.execute(
        "UPDATE stacks SET top_item_id=?2,updated_at=?3 WHERE id=?1",
        params![id, item_id, Utc::now().to_rfc3339()],
    )?;
    Ok(())
}
pub(crate) fn unstack_in(connection: &Connection, id: &str) -> Result<()> {
    if connection.execute("DELETE FROM stacks WHERE id=?1", [id])? != 1 {
        return Err(KeepframeError::Message(
            "That Stack no longer exists.".into(),
        ));
    }
    Ok(())
}
pub(crate) fn remove_from_stack_in(connection: &mut Connection, item_id: &str) -> Result<()> {
    let stack_id: Option<String> = connection
        .query_row(
            "SELECT stack_id FROM stack_items WHERE item_id=?1",
            [item_id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(stack_id) = stack_id else {
        return Ok(());
    };
    let tx = connection.transaction()?;
    let(top,count):(String,i64)=tx.query_row("SELECT top_item_id,(SELECT count(*) FROM stack_items WHERE stack_id=?1) FROM stacks WHERE id=?1",[&stack_id],|row|Ok((row.get(0)?,row.get(1)?)))?;
    if count <= 2 {
        tx.execute("DELETE FROM stacks WHERE id=?1", [stack_id])?;
    } else {
        if top == item_id {
            let replacement:String=tx.query_row("SELECT item_id FROM stack_items WHERE stack_id=?1 AND item_id<>?2 ORDER BY position,item_id LIMIT 1",params![stack_id,item_id],|row|row.get(0))?;
            tx.execute(
                "UPDATE stacks SET top_item_id=?2 WHERE id=?1",
                params![stack_id, replacement],
            )?;
        }
        tx.execute(
            "DELETE FROM stack_items WHERE stack_id=?1 AND item_id=?2",
            params![stack_id, item_id],
        )?;
    }
    tx.commit()?;
    Ok(())
}
pub(crate) fn add_to_stack_in(
    connection: &mut Connection,
    stack_id: &str,
    item_ids: Vec<String>,
) -> Result<usize> {
    let mut seen = HashSet::new();
    let ids = item_ids
        .into_iter()
        .filter(|item| seen.insert(item.clone()))
        .collect::<Vec<_>>();
    let tx = connection.transaction()?;
    let first_position: i64 = tx.query_row(
        "SELECT COALESCE(max(position),-1)+1 FROM stack_items WHERE stack_id=?1",
        [stack_id],
        |row| row.get(0),
    )?;
    let mut changed = 0;
    for (offset, item) in ids.into_iter().enumerate() {
        let position = first_position + offset as i64;
        changed+=tx.execute("INSERT INTO stack_items(stack_id,item_id,position) SELECT ?1,id,?3 FROM assets WHERE id=?2 AND trashed_at IS NULL AND NOT EXISTS(SELECT 1 FROM stack_items WHERE item_id=?2)",params![stack_id,item,position])?;
    }
    tx.commit()?;
    Ok(changed)
}

pub(crate) fn health_issues(connection: &Connection) -> Result<Vec<String>> {
    let mut issues = Vec::new();
    let invalid_primary:i64=connection.query_row("SELECT count(*) FROM (SELECT s.id,count(a.id) AS primary_count FROM sources s LEFT JOIN assets a ON a.source_id=s.id AND a.is_primary=1 GROUP BY s.id HAVING primary_count<>1)",[],|row|row.get(0))?;
    if invalid_primary > 0 {
        issues.push(format!(
            "{invalid_primary} source(s) do not have exactly one primary version"
        ));
    }
    let orphan_collections:i64=connection.query_row("SELECT count(*) FROM collection_items ci LEFT JOIN collections c ON c.id=ci.collection_id LEFT JOIN assets a ON a.id=ci.item_id WHERE c.id IS NULL OR a.id IS NULL",[],|row|row.get(0))?;
    if orphan_collections > 0 {
        issues.push(format!(
            "{orphan_collections} orphan Collection membership row(s)"
        ));
    }
    let orphan_stacks:i64=connection.query_row("SELECT count(*) FROM stack_items si LEFT JOIN stacks s ON s.id=si.stack_id LEFT JOIN assets a ON a.id=si.item_id WHERE s.id IS NULL OR a.id IS NULL",[],|row|row.get(0))?;
    if orphan_stacks > 0 {
        issues.push(format!("{orphan_stacks} orphan Stack membership row(s)"));
    }
    let duplicate_stack_items: i64 = connection.query_row(
        "SELECT count(*) FROM (SELECT item_id FROM stack_items GROUP BY item_id HAVING count(*)>1)",
        [],
        |row| row.get(0),
    )?;
    if duplicate_stack_items > 0 {
        issues.push(format!(
            "{duplicate_stack_items} item(s) belong to multiple Stacks"
        ));
    }
    let smart = connection
        .prepare("SELECT id,match_mode,rules_json FROM collections WHERE kind='smart'")?
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (id, mode, json) in smart {
        match serde_json::from_str::<Vec<SmartRule>>(&json)
            .map_err(KeepframeError::from)
            .and_then(|rules| compile_rules(&rules, &mode))
        {
            Ok(_) => {}
            Err(error) => issues.push(format!("Smart Collection {id}: {error}")),
        }
    }
    let recipes = connection
        .prepare("SELECT asset_id,recipe_json FROM develop_recipes")?
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (id, json) in recipes {
        if serde_json::from_str::<super::DevelopRecipe>(&json)
            .map_err(KeepframeError::from)
            .and_then(|recipe| recipe.validate())
            .is_err()
        {
            issues.push(format!("Catalogue item {id} has an invalid Develop recipe"));
        }
    }
    Ok(issues)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn smart_rules_are_validated_and_parameterised() {
        let (rule, values) = compile_rules(
            &[SmartRule {
                field: "keyword".into(),
                operator: "contains".into(),
                value: Value::String("%' OR 1=1 --".into()),
                second_value: None,
            }],
            "all",
        )
        .unwrap();
        assert!(rule.contains('?'));
        assert!(!rule.contains("OR 1=1"));
        assert_eq!(values.len(), 1);
        assert!(compile_rules(
            &[SmartRule {
                field: "rating".into(),
                operator: "contains".into(),
                value: Value::from(5),
                second_value: None
            }],
            "all"
        )
        .is_err());
    }

    #[test]
    fn versions_collections_stacks_and_portable_round_trip_preserve_one_source() {
        use super::super::{
            hash_file, initialise_layout, open_db, BasicAdjustments, DevelopRecipe,
        };
        use rusqlite::params;
        use std::fs;
        let root = std::env::temp_dir().join(format!("keepframe-m13-{}", Uuid::new_v4()));
        initialise_layout(&root).unwrap();
        let original = root.join("Originals/photo.png");
        fs::create_dir_all(original.parent().unwrap()).unwrap();
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            48,
            32,
            image::Rgb([70, 100, 130]),
        ))
        .save(&original)
        .unwrap();
        let before = hash_file(&original).unwrap();
        let thumbnail = root.join(".keepframe/thumbnails/source.png");
        fs::copy(&original, &thumbnail).unwrap();
        let mut connection = open_db(&root).unwrap();
        connection.execute("INSERT INTO assets(id,filename,decision,rating,title,captured_at,thumbnail_path,created_at)VALUES('primary','photo.png','keep',5,'Colour','2026-09-13T10:00:00Z',?1,'2026-09-13T10:00:00Z')",[thumbnail.to_string_lossy().as_ref()]).unwrap();
        connection.execute("INSERT INTO representations(id,asset_id,path,sha256,extension,stem,byte_size,is_raw)VALUES('representation','primary',?1,?2,'png','photo',?3,0)",params![original.to_string_lossy(),before,fs::metadata(&original).unwrap().len()]).unwrap();
        let mut recipe = DevelopRecipe::neutral();
        recipe.settings = BasicAdjustments {
            exposure: 0.8,
            ..BasicAdjustments::neutral()
        };
        recipe.advanced.detail.sharpen_amount = 32.0;
        recipe.advanced.colour_mixer.bands[1].saturation = 14.0;
        {
            let tx = connection.transaction().unwrap();
            super::super::write_develop_recipe(&tx, "primary", &recipe).unwrap();
            tx.commit().unwrap();
        }
        let current = create_version_in(
            &mut connection,
            CreateVersionRequest {
                item_id: "primary".into(),
                mode: "current".into(),
                name: Some("Print".into()),
            },
        )
        .unwrap();
        let default = create_version_in(
            &mut connection,
            CreateVersionRequest {
                item_id: "primary".into(),
                mode: "default".into(),
                name: None,
            },
        )
        .unwrap();
        let duplicate = create_version_in(
            &mut connection,
            CreateVersionRequest {
                item_id: "primary".into(),
                mode: "duplicate".into(),
                name: Some("Client Edit".into()),
            },
        )
        .unwrap();
        assert_eq!(
            connection
                .query_row("SELECT count(*) FROM sources", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            connection
                .query_row("SELECT count(*) FROM representations", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            super::super::develop_recipe_in(&connection, &current).unwrap(),
            recipe
        );
        assert_eq!(
            super::super::develop_recipe_in(&connection, &default).unwrap(),
            DevelopRecipe::neutral()
        );
        assert_eq!(
            super::super::develop_recipe_in(&connection, &duplicate).unwrap(),
            recipe
        );
        let mut sibling_recipe = recipe.clone();
        sibling_recipe.advanced.detail.sharpen_amount = 72.0;
        sibling_recipe.advanced.colour_mixer.bands[1].saturation = -18.0;
        {
            let tx = connection.transaction().unwrap();
            super::super::write_develop_recipe(&tx, &current, &sibling_recipe).unwrap();
            tx.commit().unwrap();
        }
        assert_eq!(
            super::super::develop_recipe_in(&connection, "primary").unwrap(),
            recipe
        );
        assert_eq!(
            super::super::develop_recipe_in(&connection, &current).unwrap(),
            sibling_recipe
        );
        assert_eq!(
            super::super::develop_recipe_in(&connection, &duplicate).unwrap(),
            recipe
        );
        connection
            .execute(
                "UPDATE assets SET rating=3,decision='discard' WHERE id=?1",
                [&current],
            )
            .unwrap();
        assert_eq!(
            connection
                .query_row("SELECT rating FROM assets WHERE id='primary'", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            5
        );
        let set = create_collection_set_in(&connection, "Client").unwrap();
        let manual = save_collection_in(
            &mut connection,
            SaveCollectionRequest {
                id: None,
                name: "Deliverables".into(),
                kind: "manual".into(),
                set_id: Some(set),
                match_mode: "all".into(),
                rules: vec![],
            },
        )
        .unwrap();
        assert_eq!(
            update_collection_members_in(
                &mut connection,
                &manual,
                vec![current.clone(), current.clone()],
                true
            )
            .unwrap(),
            1
        );
        let smart = save_collection_in(
            &mut connection,
            SaveCollectionRequest {
                id: None,
                name: "Edited picks".into(),
                kind: "smart".into(),
                set_id: None,
                match_mode: "all".into(),
                rules: vec![
                    SmartRule {
                        field: "rating".into(),
                        operator: "gte".into(),
                        value: Value::from(4),
                        second_value: None,
                    },
                    SmartRule {
                        field: "edited".into(),
                        operator: "is".into(),
                        value: Value::Bool(true),
                        second_value: None,
                    },
                ],
            },
        )
        .unwrap();
        assert_eq!(
            list_collections_in(&connection)
                .unwrap()
                .into_iter()
                .find(|value| value.id == smart)
                .unwrap()
                .count,
            2
        );
        let stack = create_stack_in(
            &mut connection,
            vec!["primary".into(), current.clone(), default.clone()],
            &current,
        )
        .unwrap();
        set_stack_top_in(&connection, &stack, &default).unwrap();
        set_stack_collapsed_in(&connection, &stack, false).unwrap();
        connection.execute("INSERT INTO semantic_suggestion_decisions(identity_hash,kind,decision,model_id,model_revision,member_source_ids_json,decided_at)VALUES('decision-1','burst','dismissed','model','revision','[\"primary\"]','2026-09-13T10:00:00Z')",[]).unwrap();
        assert_eq!(list_stacks_in(&connection).unwrap()[0].member_ids.len(), 3);
        assert!(delete_version_in(&connection, "primary").is_err());
        let destination = root.join("portable");
        fs::create_dir_all(&destination).unwrap();
        let portable = super::super::interoperability::export_portable_catalogue(
            &connection,
            &root,
            &destination,
        )
        .unwrap();
        connection.execute("DELETE FROM stacks", []).unwrap();
        connection.execute("DELETE FROM collections", []).unwrap();
        connection
            .execute("DELETE FROM collection_sets", [])
            .unwrap();
        connection
            .execute("DELETE FROM assets WHERE is_primary=0", [])
            .unwrap();
        connection
            .execute("DELETE FROM semantic_suggestion_decisions", [])
            .unwrap();
        assert_eq!(
            super::super::interoperability::import_portable_catalogue(&mut connection, &portable)
                .unwrap(),
            4
        );
        assert_eq!(list_versions_in(&connection, "primary").unwrap().len(), 4);
        assert_eq!(list_collections_in(&connection).unwrap().len(), 2);
        assert_eq!(list_stacks_in(&connection).unwrap()[0].top_item_id, default);
        assert_eq!(
            connection
                .query_row(
                    "SELECT decision FROM semantic_suggestion_decisions WHERE identity_hash='decision-1'",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            "dismissed"
        );
        assert_eq!(
            super::super::develop_recipe_in(&connection, &current).unwrap(),
            sibling_recipe
        );
        assert!(health_issues(&connection).unwrap().is_empty());
        assert_eq!(hash_file(&original).unwrap(), before);
        delete_version_in(&connection, &duplicate).unwrap();
        assert!(original.is_file());
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[ignore = "manual Milestone 13 organisation benchmark; run scripts/benchmark-catalogue-organisation.ps1"]
    fn catalogue_organisation_performance_checkpoint() -> Result<()> {
        use super::super::{initialise_layout, open_db};
        use std::time::Instant;
        let root = std::env::temp_dir().join(format!("keepframe-m13-benchmark-{}", Uuid::new_v4()));
        initialise_layout(&root).unwrap();
        let mut connection = open_db(&root).unwrap();
        let started = Instant::now();
        connection.execute_batch("WITH RECURSIVE n(x) AS(SELECT 0 UNION ALL SELECT x+1 FROM n WHERE x<9999) INSERT INTO assets(id,filename,decision,rating,captured_at,thumbnail_path,created_at) SELECT printf('asset-%05d',x),printf('photo-%05d.jpg',x),'undecided',x%6,'2026-01-01T00:00:00Z','thumb.jpg','2026-01-01T00:00:00Z' FROM n;WITH RECURSIVE n(x) AS(SELECT 0 UNION ALL SELECT x+1 FROM n WHERE x<9999) INSERT INTO representations(id,asset_id,path,sha256,extension,stem,byte_size,is_raw) SELECT printf('rep-%05d',x),printf('asset-%05d',x),printf('C:/bench/photo-%05d.jpg',x),printf('hash-%05d',x),'jpg',printf('photo-%05d',x),1,0 FROM n;INSERT INTO assets(id,filename,decision,rating,captured_at,thumbnail_path,created_at,source_id,is_primary,version_name,version_index) SELECT 'version-'||id,filename,decision,rating,captured_at,thumbnail_path,created_at,source_id,0,'Print',2 FROM assets WHERE is_primary=1;INSERT INTO develop_recipes(asset_id,schema_version,recipe_json,updated_at) SELECT id,2,'{}','2026-01-01T00:00:00Z' FROM assets WHERE is_primary=0 AND rating>=4;INSERT INTO tags(id,name)VALUES('benchmark-tag','Portfolio');INSERT INTO asset_tags(asset_id,tag_id) SELECT id,'benchmark-tag' FROM assets WHERE is_primary=0 AND rating>=4;").unwrap();
        let build_ms = started.elapsed().as_millis();
        let manual = save_collection_in(
            &mut connection,
            SaveCollectionRequest {
                id: None,
                name: "Ten thousand versions".into(),
                kind: "manual".into(),
                set_id: None,
                match_mode: "all".into(),
                rules: vec![],
            },
        )
        .unwrap();
        let ids = connection
            .prepare("SELECT id FROM assets WHERE is_primary=0")?
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()
            .unwrap();
        let started = Instant::now();
        assert_eq!(
            update_collection_members_in(&mut connection, &manual, ids, true).unwrap(),
            10_000
        );
        let manual_ms = started.elapsed().as_millis();
        let all_rules = vec![
            SmartRule {
                field: "rating".into(),
                operator: "gte".into(),
                value: Value::from(4),
                second_value: None,
            },
            SmartRule {
                field: "edited".into(),
                operator: "is".into(),
                value: Value::Bool(true),
                second_value: None,
            },
        ];
        let any_rules = vec![
            SmartRule {
                field: "rating".into(),
                operator: "gte".into(),
                value: Value::from(5),
                second_value: None,
            },
            SmartRule {
                field: "keyword".into(),
                operator: "contains".into(),
                value: Value::String("Portfolio".into()),
                second_value: None,
            },
        ];
        let started = Instant::now();
        let (all_sql, all_values) = compile_rules(&all_rules, "all").unwrap();
        let all_count:i64=connection.query_row(&format!("SELECT count(*) FROM assets a JOIN sources s ON s.id=a.source_id WHERE {all_sql}"),params_from_iter(all_values.iter()),|row|row.get(0)).unwrap();
        let all_ms = started.elapsed().as_millis();
        let started = Instant::now();
        let (any_sql, any_values) = compile_rules(&any_rules, "any").unwrap();
        let any_count:i64=connection.query_row(&format!("SELECT count(*) FROM assets a JOIN sources s ON s.id=a.source_id WHERE {any_sql}"),params_from_iter(any_values.iter()),|row|row.get(0)).unwrap();
        let any_ms = started.elapsed().as_millis();
        println!("M13_ORGANISATION_BENCHMARK sources=10000 items=20000 build_ms={build_ms} manual_collection_10000_ms={manual_ms} smart_all_ms={all_ms} smart_any_ms={any_ms} all_count={all_count} any_count={any_count}");
        assert_eq!(
            connection
                .query_row("SELECT count(*) FROM sources", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            10_000
        );
        assert_eq!(
            connection
                .query_row("SELECT count(*) FROM assets", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            20_000
        );
        assert!(all_count > 0 && any_count >= all_count);
        drop(connection);
        std::fs::remove_dir_all(root).unwrap();
        Ok(())
    }
}
