/// Errors loading or saving a scene.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SceneError {
    /// An independent paint transform cannot be inverted to sample its paint.
    #[error("paint transform must be finite and invertible")]
    PaintTransform,
    /// An I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// `scene.json` could not be parsed.
    #[error("scene JSON error: {0}")]
    Json(#[from] serde_json::Error),
    /// A referenced resource blob is missing.
    #[error("missing resource {0}")]
    MissingResource(crate::ResourceHash),
    /// An image's declared encoding cannot decode its bytes.
    #[error("invalid image encoding {0:?}")]
    InvalidImageEncoding(crate::ImageEncoding),
    /// A layer samples a backdrop group the scene does not declare.
    #[error("layer samples unknown backdrop group {0}")]
    UnknownBackdropGroup(u32),
    /// A backdrop-group member layer has no clip.
    #[error("backdrop group {0} member layer has no clip")]
    BackdropMemberUnclipped(u32),
    /// A layer has a `backdrop_effect` without a `backdrop` group.
    #[error("layer has a backdrop effect but no backdrop group")]
    BackdropEffectWithoutGroup,
    /// A `backdrop_effect` parameter is non-finite or out of range.
    #[error("invalid backdrop effect: {0}")]
    InvalidBackdropEffect(&'static str),
    /// A `projection` or a `Motion::Tilt` is misplaced: on the scene root,
    /// a tilt motion without a projection, or a rotation motion on a
    /// projective layer.
    #[error("invalid projection: {0}")]
    InvalidProjection(&'static str),
    /// A text layer's source or items are malformed.
    #[error("invalid text layer: {0}")]
    InvalidText(&'static str),
    /// The `features` set stored in `scene.json` does not match the features
    /// recomputed from the layer tree.
    #[error("stored features {declared:?} do not match recomputed {computed:?}")]
    FeatureMismatch {
        /// The feature set stored in the file.
        declared: Vec<crate::Feature>,
        /// The feature set recomputed from the layer tree.
        computed: Vec<crate::Feature>,
    },
}
