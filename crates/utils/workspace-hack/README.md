# Workspace Hack

This is a dummy crate which imports any dependencies and feature combinations needed to fix cargo's feature unification for the workspace. Without this crate, the workspace does not build properly even though every *member* of the workspace does.
