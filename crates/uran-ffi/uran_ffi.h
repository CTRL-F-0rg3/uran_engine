/* uran_ffi.h — nagłówek ABI dla wtyczek C/C++ i P/Invoke C#.
 *
 * Silnik Uran (Rust) eksportuje funkcje matematyczne, a wtyczka (C#/C++)
 * eksportuje jeden symbol `uran_plugin_get`, który zwraca `const UranPlugin*`.
 * Rust woła wtyczkę przez tę tabelę wskaźników (patrz `PluginHost`).
 */
#ifndef URAN_FFI_H
#define URAN_FFI_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Typy zgodne z ABI (repr(C) po stronie Rusta). */
typedef struct CVec2 { float x, y; } CVec2;
typedef struct CRect { float x, y, w, h; } CRect;   /* x,y = róg min, w,h = rozmiar */
typedef struct CColor { float r, g, b, a; } CColor; /* kanały 0..1 */

/* Eksportowane przez silnik (Rust). */
CVec2  uran_vec2_add(CVec2 a, CVec2 b);
float  uran_vec2_length(CVec2 v);
CVec2  uran_vec2_normalized(CVec2 v);
int32_t uran_rect_contains(CRect r, CVec2 p);
CColor uran_color_from_hex(uint32_t hex);

/* Kontrakt wtyczki — implementuje wtyczka (C#/C++). */
typedef struct UranPlugin {
    const char* (*name)(void);
    void* (*create)(void);
    void  (*destroy)(void* state);
    void  (*init)(void* state);
    void  (*update)(void* state, float dt);
} UranPlugin;

/* Wtyczka eksportuje ten symbol:
 *   const UranPlugin* uran_plugin_get(void);
 */

#ifdef __cplusplus
}
#endif

#endif /* URAN_FFI_H */
