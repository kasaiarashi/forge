// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

#pragma once

#include "ISourceControlOperation.h"

/**
 * Maps UE source control operations to Forge CLI commands:
 *
 *   FCheckOut       -> forge lock <file>
 *   FCheckIn        -> forge snapshot -m "..." && forge unlock && forge push
 *   FRevert         -> restore from snapshot + forge unlock
 *   FSync           -> forge pull
 *   FUpdateStatus   -> forge status --json + forge locks --json
 *   FMarkForAdd     -> forge add <file>
 *   FDelete         -> mark for deletion
 *   FGetFileHistory -> forge log --file <file> --json
 *   FConnect        -> verify forge workspace exists
 */

// No custom operation classes needed yet — we use the built-in UE operation
// types (FCheckOut, FCheckIn, etc.) and dispatch them in the provider's
// Execute() method based on operation name.
