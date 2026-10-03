use crate::Error;
use std::io::{Read, Seek, SeekFrom};

pub(super) struct Input<R> {
    inner: R,
    pub length: u64,
    pub position: u64,
    pub end: u64,
}

impl<R: Read + Seek> Input<R> {
    pub fn new(mut inner: R, max_bytes: u64) -> Result<Self, Error> {
        let length = inner.seek(SeekFrom::End(0))?;
        if length > max_bytes {
            return Err(Error::Limit("OpenVDB input bytes"));
        }
        inner.seek(SeekFrom::Start(0))?;
        Ok(Self { inner, length, position: 0, end: length })
    }
    pub fn seek(&mut self, position: u64) -> Result<(), Error> {
        if position > self.end {
            return Err(Error::Invalid("OpenVDB offset outside section"));
        }
        self.inner.seek(SeekFrom::Start(position))?;
        self.position = position;
        Ok(())
    }
    pub fn bytes<const N: usize>(&mut self) -> Result<[u8; N], Error> {
        let mut out = [0; N];
        self.read(&mut out)?;
        Ok(out)
    }
    pub fn read(&mut self, bytes: &mut [u8]) -> Result<(), Error> {
        let next = self.position.checked_add(bytes.len() as u64).ok_or(Error::Limit("OpenVDB offset overflow"))?;
        if next > self.end {
            return Err(Error::Invalid("truncated OpenVDB section"));
        }
        self.inner.read_exact(bytes)?;
        self.position = next;
        Ok(())
    }
    pub fn u8(&mut self) -> Result<u8, Error> {
        Ok(self.bytes::<1>()?[0])
    }
    pub fn u32(&mut self) -> Result<u32, Error> {
        Ok(u32::from_le_bytes(self.bytes()?))
    }
    pub fn u64(&mut self) -> Result<u64, Error> {
        Ok(u64::from_le_bytes(self.bytes()?))
    }
    pub fn i64(&mut self) -> Result<i64, Error> {
        Ok(i64::from_le_bytes(self.bytes()?))
    }
    pub fn f64(&mut self) -> Result<f64, Error> {
        Ok(f64::from_le_bytes(self.bytes()?))
    }
    pub fn coord(&mut self) -> Result<[i32; 3], Error> {
        Ok([self.u32()? as i32, self.u32()? as i32, self.u32()? as i32])
    }
    pub fn string(&mut self, max: usize) -> Result<String, Error> {
        let n = self.u32()? as usize;
        if n > max {
            return Err(Error::Limit("OpenVDB string length"));
        }
        let mut bytes = vec![0; n];
        self.read(&mut bytes)?;
        String::from_utf8(bytes).map_err(|_| Error::Invalid("OpenVDB UTF-8 string"))
    }
    pub fn metadata(&mut self) -> Result<(), Error> {
        let start = self.position;
        let count = self.u32()?;
        if count > 4096 {
            return Err(Error::Limit("OpenVDB metadata entries"));
        }
        for _ in 0..count {
            self.string(4096)?;
            self.string(4096)?;
            let n = u64::from(self.u32()?);
            let next = self.position.checked_add(n).ok_or(Error::Limit("OpenVDB metadata size"))?;
            if next - start > 1 << 20 {
                return Err(Error::Limit("OpenVDB metadata bytes"));
            }
            self.seek(next)?;
        }
        Ok(())
    }
    pub fn mask(&mut self, n: usize) -> Result<Vec<u64>, Error> {
        let mut out = vec![0; n / 64];
        for x in &mut out {
            *x = self.u64()?;
        }
        Ok(out)
    }
}
