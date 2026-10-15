//! The fold: Jetpack WindowManager's `FoldingFeature`, cached by the Java side as it changes
//! (`Bridge.fold`), read here.

use crate::device::Fold;

pub(crate) fn fold() -> Option<Fold> {
    let v = super::device::floats("fold")?;
    crate::device::fold_from_floats(&v)
}
