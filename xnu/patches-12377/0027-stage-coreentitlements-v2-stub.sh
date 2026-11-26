#!/bin/bash
# Build xnu-12377 for BERGAMOTA without a KDK. libkern/amfi/amfi.h includes <CoreEntitlements/V2/API.h> and
# <CoreEntitlements/V2/Kernel.h>, which are not in the open-source tree (dxb copies them from the KDK in
# patches/26.0/00_setup_coreentitlements.sh). Every xnu call through the V2 API (libCoreEntitlements->...) is
# under CODE_SIGNING_MONITOR or PMAP_CS_INCLUDE_CODE_SIGNING, both off for BERGAMOTA, and BERGAMOTA loads no AMFI kext
# that would register an implementation. So only the type names are needed: opaque structs and the return enum.
# When the real headers are present (KDK build) this script does nothing.
set -euo pipefail

[[ "${MACHINE_CONFIG:-}" == BERGAMOTA ]] || exit 0
[[ -f libkern/libkern/amfi/amfi.h ]] || exit 1
v2=EXTERNAL_HEADERS/CoreEntitlements/V2
if [[ -f "$v2/API.h" && -f "$v2/Kernel.h" ]]; then
    echo "CoreEntitlements V2 headers present (KDK): no stub"
    exit 0
fi
mkdir -p "$v2"

cat > "$v2/API.h" <<'H'
/* BERGAMOTA stand-in for the KDK-only CoreEntitlements V2 API: type names only (no implementation exists in a
 * BERGAMOTA kernel; every caller is compiled out with CODE_SIGNING_MONITOR / PMAP_CS off). */
#ifndef _BERGAMOTA_COREENTITLEMENTS_V2_API_H
#define _BERGAMOTA_COREENTITLEMENTS_V2_API_H

#include <CoreEntitlements/CoreEntitlements.h>

typedef enum {
	kCEReturnSuccess = 0,
	kCEReturnError,
} CEReturn_t;

typedef enum {
	kCEContextTypeInvalid = 0,
	kCEContextTypeProvisioningProfile,
	kCEContextTypeProvisioningProfileEntitlements,
} CEContextType_t;

typedef struct CEContext CEContext_t;
typedef struct CEElement CEElement_t;
typedef CEBuffer CEBuffer_t;                    /* CEBuffer: CoreEntitlements/Runtime.h (V1) */

#endif /* _BERGAMOTA_COREENTITLEMENTS_V2_API_H */
H

cat > "$v2/Kernel.h" <<'H'
/* BERGAMOTA stand-in for the KDK-only CoreEntitlements V2 kernel table (see API.h). Opaque: nothing in a BERGAMOTA
 * kernel registers or calls it. */
#ifndef _BERGAMOTA_COREENTITLEMENTS_V2_KERNEL_H
#define _BERGAMOTA_COREENTITLEMENTS_V2_KERNEL_H

#include <CoreEntitlements/V2/API.h>

typedef struct CEKernelAPI CEKernelAPI_t;

#endif /* _BERGAMOTA_COREENTITLEMENTS_V2_KERNEL_H */
H
echo "Staged CoreEntitlements V2 type stubs for BERGAMOTA (no KDK)"
