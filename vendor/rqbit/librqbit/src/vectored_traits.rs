use rqbit_runtime::Cx;
use std::{future::poll_fn, io::IoSliceMut, pin::Pin, task::Poll};

use rqbit_runtime::io::AsyncRead;

pub trait AsyncReadVectored: AsyncRead + Unpin {
    fn poll_read_vectored(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        vec: &mut [IoSliceMut<'_>],
    ) -> Poll<std::io::Result<usize>>;
}

impl<T: ?Sized + AsyncReadVectored> AsyncReadVectored for &mut T {
    fn poll_read_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        vec: &mut [IoSliceMut<'_>],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut **self).poll_read_vectored(cx, vec)
    }
}

impl<T: ?Sized + AsyncReadVectored> AsyncReadVectored for Box<T> {
    fn poll_read_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        vec: &mut [IoSliceMut<'_>],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut **self).poll_read_vectored(cx, vec)
    }
}

pub trait AsyncReadVectoredExt {
    async fn read_vectored(
        &mut self,
        owner: &Cx,
        vec: &mut [IoSliceMut<'_>],
    ) -> std::io::Result<usize>;
}

impl<T: AsyncReadVectored> AsyncReadVectoredExt for T {
    async fn read_vectored(
        &mut self,
        owner: &Cx,
        vec: &mut [IoSliceMut<'_>],
    ) -> std::io::Result<usize> {
        owner
            .checkpoint()
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        poll_fn(|cx| Pin::new(&mut *self).poll_read_vectored(cx, vec)).await
    }
}

impl AsyncReadVectored for rqbit_runtime::net::OwnedReadHalf {
    fn poll_read_vectored(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        vec: &mut [IoSliceMut<'_>],
    ) -> Poll<std::io::Result<usize>> {
        rqbit_runtime::io::AsyncReadVectored::poll_read_vectored(self, cx, vec)
    }
}

impl AsyncReadVectored for rqbit_runtime::net::BoundReadHalf {
    fn poll_read_vectored(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        vec: &mut [IoSliceMut<'_>],
    ) -> Poll<std::io::Result<usize>> {
        rqbit_runtime::io::AsyncReadVectored::poll_read_vectored(self, cx, vec)
    }
}
