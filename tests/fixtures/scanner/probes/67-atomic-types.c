typedef int scalar_type;
struct payload { int value; };
enum status { state_ok };
_Atomic(int) plain_atomic;
_Atomic(unsigned long) initialized_atomic = 2;
static _Atomic(scalar_type) named_atomic;
_Atomic(int *) pointer_atomic;
_Atomic(struct payload) aggregate_atomic;
_Atomic(enum status) enum_atomic;
_Atomic /* type gap */ ( /* operand gap */ int /* close gap */ )
split_atomic;
_Atomic(
int
) multiline_atomic;
_Atomic int qualified_atomic;
typedef _Atomic(int) atomic_alias;
typedef _Atomic(scalar_type) named_atomic_alias;
int ordinary_after_alias;
extern _Atomic(int) extern_atomic;
int after_extern_atomic;
_Atomic(int) factory(void);
int after_factory;
int function(_Atomic(int) parameter) {
    _Atomic(int) local_atomic;
    static _Atomic(int) local_static_atomic;
    return parameter;
}
int after_function;
struct holder {
    _Atomic(int) member_atomic;
    _Atomic(scalar_type) named_member;
    int after_member;
};
_Atomic(int) comma_one, comma_two = 1;
_Atomic(int) initializer_atomic = sizeof(_Atomic(int));
int after_initializer;
