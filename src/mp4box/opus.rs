use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use serde::Serialize;
use std::io::{Read, Seek, Write};

use crate::mp4box::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OpusBox {
    pub data_reference_index: u16,
    pub channelcount: u16,
    pub samplesize: u16,

    #[serde(with = "value_u32")]
    pub samplerate: FixedPointU16,
    pub dops: DOpsBox,
}

impl Default for OpusBox {
    fn default() -> Self {
        Self {
            data_reference_index: 0,
            channelcount: 2,
            samplesize: 16,
            samplerate: FixedPointU16::new(48000),
            dops: DOpsBox::default(),
        }
    }
}

impl OpusBox {
    pub fn new(config: &OpusConfig) -> Self {
        let channel_mapping_table = if config.channel_mapping_family != 0 {
            Some(ChannelMappingTable {
                stream_count: config.stream_count.unwrap_or(0),
                coupled_count: config.coupled_count.unwrap_or(0),
                channel_mapping: config.channel_mapping.clone().unwrap_or_default(),
            })
        } else {
            None
        };
        Self {
            data_reference_index: 1,
            channelcount: config.output_channel_count as u16,
            samplesize: 16,
            samplerate: FixedPointU16::new(48000),
            dops: DOpsBox {
                version: 0,
                output_channel_count: config.output_channel_count,
                pre_skip: config.pre_skip,
                input_sample_rate: config.input_sample_rate,
                output_gain: config.output_gain,
                channel_mapping_family: config.channel_mapping_family,
                channel_mapping_table,
            },
        }
    }

    pub fn get_type(&self) -> BoxType {
        BoxType::OpusBox
    }

    pub fn get_size(&self) -> u64 {
        HEADER_SIZE + 8 + 20 + self.dops.box_size()
    }
}

impl Mp4Box for OpusBox {
    fn box_type(&self) -> BoxType {
        self.get_type()
    }

    fn box_size(&self) -> u64 {
        self.get_size()
    }

    fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string(&self).unwrap())
    }

    fn summary(&self) -> Result<String> {
        let s = format!(
            "channel_count={} sample_size={} sample_rate={}",
            self.channelcount,
            self.samplesize,
            self.samplerate.value()
        );
        Ok(s)
    }
}

impl<R: Read + Seek> ReadBox<&mut R> for OpusBox {
    fn read_box(reader: &mut R, size: u64) -> Result<Self> {
        let start = box_start(reader)?;

        reader.read_u32::<BigEndian>()?; // reserved
        reader.read_u16::<BigEndian>()?; // reserved
        let data_reference_index = reader.read_u16::<BigEndian>()?;
        reader.read_u16::<BigEndian>()?; // version
        reader.read_u16::<BigEndian>()?; // reserved
        reader.read_u32::<BigEndian>()?; // reserved
        let channelcount = reader.read_u16::<BigEndian>()?;
        let samplesize = reader.read_u16::<BigEndian>()?;
        reader.read_u32::<BigEndian>()?; // pre-defined, reserved
        let samplerate = FixedPointU16::new_raw(reader.read_u32::<BigEndian>()?);

        let header = BoxHeader::read(reader)?;
        let BoxHeader { name, size: s } = header;
        if s > size {
            return Err(Error::InvalidData(
                "opus box contains a box with a larger size than it",
            ));
        }
        if name != BoxType::DOpsBox {
            return Err(Error::InvalidData("opus box must contain dOps box"));
        }
        let dops = DOpsBox::read_box(reader, s)?;

        skip_bytes_to(reader, start + size)?;

        Ok(OpusBox {
            data_reference_index,
            channelcount,
            samplesize,
            samplerate,
            dops,
        })
    }
}

impl<W: Write> WriteBox<&mut W> for OpusBox {
    fn write_box(&self, writer: &mut W) -> Result<u64> {
        let size = self.box_size();
        BoxHeader::new(self.box_type(), size).write(writer)?;

        writer.write_u32::<BigEndian>(0)?; // reserved
        writer.write_u16::<BigEndian>(0)?; // reserved
        writer.write_u16::<BigEndian>(self.data_reference_index)?;

        writer.write_u64::<BigEndian>(0)?; // reserved
        writer.write_u16::<BigEndian>(self.channelcount)?;
        writer.write_u16::<BigEndian>(self.samplesize)?;
        writer.write_u32::<BigEndian>(0)?; // reserved
        writer.write_u32::<BigEndian>(self.samplerate.raw_value())?;

        self.dops.write_box(writer)?;

        Ok(size)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DOpsBox {
    pub version: u8,
    pub output_channel_count: u8,
    pub pre_skip: u16,
    pub input_sample_rate: u32,
    pub output_gain: i16,
    pub channel_mapping_family: u8,
    pub channel_mapping_table: Option<ChannelMappingTable>,
}

impl Default for DOpsBox {
    fn default() -> Self {
        Self {
            version: 0,
            output_channel_count: 2,
            pre_skip: 0,
            input_sample_rate: 48000,
            output_gain: 0,
            channel_mapping_family: 0,
            channel_mapping_table: None,
        }
    }
}

impl DOpsBox {
    fn base_size() -> u64 {
        HEADER_SIZE + 11
    }
}

impl Mp4Box for DOpsBox {
    fn box_type(&self) -> BoxType {
        BoxType::DOpsBox
    }

    fn box_size(&self) -> u64 {
        let mut size = Self::base_size();
        if let Some(ref table) = self.channel_mapping_table {
            size += 2 + table.channel_mapping.len() as u64;
        }
        size
    }

    fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string(&self).unwrap())
    }

    fn summary(&self) -> Result<String> {
        let s = format!(
            "version={} output_channel_count={} pre_skip={} input_sample_rate={} output_gain={} channel_mapping_family={}",
            self.version,
            self.output_channel_count,
            self.pre_skip,
            self.input_sample_rate,
            self.output_gain,
            self.channel_mapping_family,
        );
        Ok(s)
    }
}

impl<R: Read + Seek> ReadBox<&mut R> for DOpsBox {
    fn read_box(reader: &mut R, size: u64) -> Result<Self> {
        let start = box_start(reader)?;

        let version = reader.read_u8()?;
        let output_channel_count = reader.read_u8()?;
        let pre_skip = reader.read_u16::<BigEndian>()?;
        let input_sample_rate = reader.read_u32::<BigEndian>()?;
        let output_gain = reader.read_i16::<BigEndian>()?;
        let channel_mapping_family = reader.read_u8()?;

        let channel_mapping_table = if channel_mapping_family != 0 {
            let stream_count = reader.read_u8()?;
            let coupled_count = reader.read_u8()?;
            let mut channel_mapping = vec![0u8; output_channel_count as usize];
            reader.read_exact(&mut channel_mapping)?;
            Some(ChannelMappingTable {
                stream_count,
                coupled_count,
                channel_mapping,
            })
        } else {
            None
        };

        skip_bytes_to(reader, start + size)?;

        Ok(DOpsBox {
            version,
            output_channel_count,
            pre_skip,
            input_sample_rate,
            output_gain,
            channel_mapping_family,
            channel_mapping_table,
        })
    }
}

impl<W: Write> WriteBox<&mut W> for DOpsBox {
    fn write_box(&self, writer: &mut W) -> Result<u64> {
        let size = self.box_size();
        BoxHeader::new(self.box_type(), size).write(writer)?;

        writer.write_u8(self.version)?;
        writer.write_u8(self.output_channel_count)?;
        writer.write_u16::<BigEndian>(self.pre_skip)?;
        writer.write_u32::<BigEndian>(self.input_sample_rate)?;
        writer.write_i16::<BigEndian>(self.output_gain)?;
        writer.write_u8(self.channel_mapping_family)?;

        if let Some(ref table) = self.channel_mapping_table {
            writer.write_u8(table.stream_count)?;
            writer.write_u8(table.coupled_count)?;
            writer.write_all(&table.channel_mapping)?;
        }

        Ok(size)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChannelMappingTable {
    pub stream_count: u8,
    pub coupled_count: u8,
    pub channel_mapping: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mp4box::BoxHeader;
    use std::io::Cursor;

    #[test]
    fn test_opus() {
        let src_box = OpusBox {
            data_reference_index: 1,
            channelcount: 2,
            samplesize: 16,
            samplerate: FixedPointU16::new(48000),
            dops: DOpsBox {
                version: 0,
                output_channel_count: 2,
                pre_skip: 312,
                input_sample_rate: 48000,
                output_gain: 0,
                channel_mapping_family: 0,
                channel_mapping_table: None,
            },
        };
        let mut buf = Vec::new();
        src_box.write_box(&mut buf).unwrap();
        assert_eq!(buf.len(), src_box.box_size() as usize);

        let mut reader = Cursor::new(&buf);
        let header = BoxHeader::read(&mut reader).unwrap();
        assert_eq!(header.name, BoxType::OpusBox);
        assert_eq!(src_box.box_size(), header.size);

        let dst_box = OpusBox::read_box(&mut reader, header.size).unwrap();
        assert_eq!(src_box, dst_box);
    }

    #[test]
    fn test_opus_with_channel_mapping() {
        let src_box = OpusBox {
            data_reference_index: 1,
            channelcount: 6,
            samplesize: 16,
            samplerate: FixedPointU16::new(48000),
            dops: DOpsBox {
                version: 0,
                output_channel_count: 6,
                pre_skip: 312,
                input_sample_rate: 48000,
                output_gain: 0,
                channel_mapping_family: 1,
                channel_mapping_table: Some(ChannelMappingTable {
                    stream_count: 4,
                    coupled_count: 2,
                    channel_mapping: vec![0, 4, 1, 2, 3, 5],
                }),
            },
        };
        let mut buf = Vec::new();
        src_box.write_box(&mut buf).unwrap();
        assert_eq!(buf.len(), src_box.box_size() as usize);

        let mut reader = Cursor::new(&buf);
        let header = BoxHeader::read(&mut reader).unwrap();
        assert_eq!(header.name, BoxType::OpusBox);
        assert_eq!(src_box.box_size(), header.size);

        let dst_box = OpusBox::read_box(&mut reader, header.size).unwrap();
        assert_eq!(src_box, dst_box);
    }
}
