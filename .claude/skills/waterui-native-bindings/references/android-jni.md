# Android JNI Implementation Guide

Complete step-by-step for adding a new component to the Android backend.

## Step 1: Kotlin Struct (`FfiStructs.kt`)

Location: `runtime/src/main/java/dev/waterui/android/runtime/FfiStructs.kt`

```kotlin
data class FooStruct(
    val contentPtr: Long,
    val value: Int,
    val handlerPtr: Long
)
```

## Step 2: JNI Declarations (`WatcherJni.kt`)

Location: `runtime/src/main/java/dev/waterui/android/ffi/WatcherJni.kt`

```kotlin
@JvmStatic external fun fooId(): TypeIdStruct
@JvmStatic external fun forceAsFoo(viewPtr: Long): FooStruct
@JvmStatic external fun callFoo(handlerPtr: Long, envPtr: Long)
@JvmStatic external fun dropFoo(ptr: Long)
```

## Step 3: NativeBindings Wrappers (`NativeBindings.kt`)

Location: `runtime/src/main/java/dev/waterui/android/runtime/NativeBindings.kt`

```kotlin
fun waterui_foo_id(): WuiTypeId = WatcherJni.fooId().toTypeId()
fun waterui_force_as_foo(ptr: Long): FooStruct = WatcherJni.forceAsFoo(ptr)
fun waterui_call_foo(ptr: Long, env: Long) = WatcherJni.callFoo(ptr, env)
fun waterui_drop_foo(ptr: Long) = WatcherJni.dropFoo(ptr)
```

## Step 4: C++ Symbol Table (`waterui_jni.cpp`)

Location: `runtime/src/main/cpp/waterui_jni.cpp`

Find `WATCHER_SYMBOL_LIST` macro (~line 27) and add:

```cpp
X(waterui_foo_id)        \
X(waterui_force_as_foo)  \
X(waterui_call_foo)      \
X(waterui_drop_foo)
```

**Important:** Check for existing entries to avoid duplicates (causes compile error).

## Step 5: Type ID Function (`waterui_jni.cpp`)

Find `DEFINE_TYPE_ID_FN` section (~line 1560) and add:

```cpp
DEFINE_TYPE_ID_FN(fooId, waterui_foo_id)
```

## Step 6: JNI Implementations (`waterui_jni.cpp`)

```cpp
JNIEXPORT jobject JNICALL
Java_dev_waterui_android_ffi_WatcherJni_forceAsFoo(JNIEnv *env, jclass,
                                                    jlong viewPtr) {
  auto data = g_sym.waterui_force_as_foo(jlong_to_ptr<WuiAnyView>(viewPtr));
  jclass cls = env->FindClass("dev/waterui/android/runtime/FooStruct");
  jmethodID ctor = env->GetMethodID(cls, "<init>", "(JIJ)V");
  jobject obj = env->NewObject(cls, ctor,
                               ptr_to_jlong(data.content),
                               static_cast<jint>(data.value),
                               ptr_to_jlong(data.handler));
  env->DeleteLocalRef(cls);
  return obj;
}

JNIEXPORT void JNICALL
Java_dev_waterui_android_ffi_WatcherJni_callFoo(JNIEnv *, jclass,
                                                 jlong handlerPtr, jlong envPtr) {
  g_sym.waterui_call_foo(jlong_to_ptr<WuiFooHandler>(handlerPtr),
                         jlong_to_ptr<WuiEnv>(envPtr));
}

JNIEXPORT void JNICALL
Java_dev_waterui_android_ffi_WatcherJni_dropFoo(JNIEnv *, jclass, jlong ptr) {
  g_sym.waterui_drop_foo(jlong_to_ptr<WuiFoo>(ptr));
}
```

## Step 7: Component Renderer

Location: `runtime/src/main/java/dev/waterui/android/components/FooComponent.kt`

```kotlin
package dev.waterui.android.components

import android.widget.FrameLayout
import dev.waterui.android.runtime.*

private val fooTypeId: WuiTypeId by lazy {
    NativeBindings.waterui_foo_id()
}

private val fooRenderer = WuiRenderer { context, node, env, registry ->
    val data = NativeBindings.waterui_force_as_foo(node.rawPtr)
    val container = FrameLayout(context)

    // Inflate content
    if (data.contentPtr != 0L) {
        val child = inflateAnyView(context, data.contentPtr, env, registry)
        container.addView(child)
        container.setTag(TAG_STRETCH_AXIS, child.getWuiStretchAxis())
    }

    // Implementation...

    // Cleanup
    container.disposeWith {
        if (data.contentPtr != 0L) {
            NativeBindings.waterui_drop_anyview(data.contentPtr)
        }
        NativeBindings.waterui_drop_foo(data.handlerPtr)
    }

    container
}

internal fun RegistryBuilder.registerWuiFoo() {
    register({ fooTypeId }, fooRenderer)
}
```

## Step 8: Registration (`RenderRegistry.kt`)

Location: `runtime/src/main/java/dev/waterui/android/runtime/RenderRegistry.kt`

Add in the registration block:
```kotlin
registerWuiFoo()
```

## JNI Signature Reference

| Java Type | Signature |
|-----------|-----------|
| void | V |
| boolean | Z |
| int | I |
| long | J |
| float | F |
| double | D |
| Object | Lpath/to/Class; |

Example: `(JIJ)V` = `(long, int, long) -> void`

## Common Patterns

### Tagged Union (Enum)

C header structure:
```c
typedef enum { WuiFoo_A, WuiFoo_B } WuiFoo_Tag;
typedef struct { WuiFoo_Tag tag; union { ... a; ... b; }; } WuiFoo;
```

JNI access:
```cpp
switch (data.value.tag) {
  case WuiFoo_A:
    // use data.value.a.field
    break;
  case WuiFoo_B:
    // use data.value.b.field
    break;
}
```

### Nested Struct Access

If struct has nested fields like `WuiIgnoreSafeArea { edges: WuiEdgeSet }`:
```cpp
// Wrong: data.value.top
// Right: data.value.edges.top
```

### Pointer Type Names

Check `waterui.h` for exact type names:
- `WuiComputed_Color` not `WuiComputed_WuiColor`
- `WuiGesture_Then_Body` has `first` and `then` fields (not `then_`)
