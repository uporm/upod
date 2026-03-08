use crate::business::workspace_tag::tag_dao::Tag;
use crate::utils::id::Id;
use crate::web::ts_str::to_str;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TagReq {
    pub tag_type: i32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TagCreateReq {
    pub tag_type: i32,
    pub name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TagUpdateReq {
    pub name: String,
}

impl From<(u64, u64, TagCreateReq)> for Tag {
    fn from((tenant_id, workspace_id, tag_req): (u64, u64, TagCreateReq)) -> Self {
        Self {
            id: Id::next_id().unwrap_or_default(),
            tenant_id,
            workspace_id,
            tag_type: tag_req.tag_type,
            name: tag_req.name,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TagResp {
    #[serde(serialize_with = "to_str")]
    pub id: u64,
    pub name: String,
}

impl From<Tag> for TagResp {
    fn from(tag: Tag) -> Self {
        Self {
            id: tag.id,
            name: tag.name,
        }
    }
}
