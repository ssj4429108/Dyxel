#![no_std]
#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(improper_ctypes)]
#![allow(clippy::all)]

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

extern "C" {
    pub static m3Err_none: M3Result;
    pub static m3Err_mallocFailed: M3Result;
    pub static m3Err_functionLookupFailed: M3Result;
    pub static m3Err_trapOutOfBoundsMemoryAccess: M3Result;
    pub static m3Err_trapDivisionByZero: M3Result;
    pub static m3Err_trapIntegerOverflow: M3Result;
    pub static m3Err_trapIntegerConversion: M3Result;
    pub static m3Err_trapIndirectCallTypeMismatch: M3Result;
    pub static m3Err_trapTableIndexOutOfRange: M3Result;
    pub static m3Err_trapExit: M3Result;
    pub static m3Err_trapAbort: M3Result;
    pub static m3Err_trapUnreachable: M3Result;
    pub static m3Err_trapStackOverflow: M3Result;
}
