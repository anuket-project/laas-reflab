//! Copyright (c) 2023 University of New Hampshire
//! SPDX-License-Identifier: MIT

use pyo3::{
    conversion::IntoPy,
    prelude::*,
    types::{IntoPyDict, PyAny, PyString, PyTuple},
};
use std::{collections::HashMap, marker::PhantomData};

pub struct PythonBuilder<Module>
where
    Module: ModuleInitializer,
{
    args: Vec<Box<dyn DynIntoPy>>,
    kwargs: HashMap<String, Box<dyn DynIntoPy>>,
    func: String,
    base: PhantomData<Module>,
}

/// this can be rewritten in the future to not be used,
/// currently this does the conversion late when we hold the GIL
/// in the builder, but we could instead acquire the GIL and release it
/// every time and just pass around Py<PyAny> instead of
/// the (basically) Box<dyn IntoPy<Py<PyAny>>> that this tries to do
/// here
pub trait DynIntoPy {
    fn into_py(&mut self, py: Python<'_>) -> Py<PyAny>;
}

struct DynIntoPyHolder<T>
where
    T: IntoPy<Py<PyAny>>,
{
    content: Option<Box<T>>,
}

impl<T> DynIntoPy for DynIntoPyHolder<T>
where
    T: IntoPy<Py<PyAny>>,
{
    fn into_py(&mut self, py: Python<'_>) -> Py<PyAny> {
        //self.content.into_py(py)
        self.content.take().unwrap().into_py(py)
    }
}

impl<T> DynIntoPyHolder<T>
where
    T: IntoPy<Py<PyAny>>,
{
    pub fn new(t: T) -> Box<Self> {
        Box::new(Self {
            content: Some(Box::new(t)),
        })
    }
}

impl<Module> PythonBuilder<Module>
where
    Module: ModuleInitializer,
{
    pub fn command<S>(s: S) -> PythonBuilder<Module>
    where
        S: Into<String>,
    {
        Self {
            func: s.into(),
            args: Vec::new(),
            kwargs: HashMap::new(),
            base: PhantomData,
        }
    }

    pub fn arg<A>(mut self, a: A) -> Self
    where
        A: IntoPy<Py<PyAny>> + 'static,
    {
        self.args.push(DynIntoPyHolder::new(a));
        self
    }

    pub fn kwarg<S, A>(mut self, kw: S, a: A) -> Self
    where
        A: IntoPy<Py<PyAny>> + 'static,
        S: Into<String>,
    {
        self.kwargs.insert(kw.into(), DynIntoPyHolder::new(a));
        self
    }

    pub fn run_and<F, T>(self, extractor: F) -> Result<T, PyErr>
    where
        F: FnOnce(&PyAny) -> T,
        T: 'static,
    {
        let o = Python::with_gil(|py| {
            let psn = PyString::new(py, self.func.as_str());
            let args: Vec<Py<PyAny>> = self.args.into_iter().map(|mut e| e.into_py(py)).collect();
            let args = PyTuple::new(py, args.as_slice());

            let kwargs: HashMap<String, Py<PyAny>> = self
                .kwargs
                .into_iter()
                .map(|(s, mut a)| (s, a.into_py(py)))
                .collect();
            let res = Module::init(py).call_method(psn, args, Some(kwargs.into_py_dict(py)));

            res.map(extractor)
        });

        o
    }

    pub fn run(self) -> Result<(), PyErr> {
        self.run_and(|_| ())
    }
}

pub trait ModuleInitializer {
    fn init(py: Python<'_>) -> &PyAny;
}
