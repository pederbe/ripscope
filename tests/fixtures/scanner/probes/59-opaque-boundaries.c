#if A
OPAQUE
#else
int global;
#endif
;
struct s {
    int member;
    TAIL
};

typedef int T;
#if A
_Atomic T atomic_global
#endif
;
#if A
_Thread_local T thread_global
#endif
;
#if A
__thread T gnu_thread_global
#endif
;
struct AtomicMember {
    _Atomic T atomic_member
};
struct AttributeMember {
    __attribute__((aligned(16))) T attributed_member
};
struct AtomicFormMember {
    _Atomic(T) atomic_form_member
};

EXTERN T macro_qualified_global
#if A
INIT(0);
#else
INIT(1);
#endif
struct MacroType {
    TYPEOF(T) typed_member
};
struct MacroQualifier {
    EXTERN
    T
    qualified_member
};

/* Opaque-member limit: expansion and a missing semicolon can hide a type. */
#define TYPE_FROM(x) __typeof__(x)
struct NumericMacroType {
    TYPE_FROM(0) macro_type_member
};
