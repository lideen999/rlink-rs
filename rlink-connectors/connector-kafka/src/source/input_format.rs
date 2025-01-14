use std::time::Duration;

use rdkafka::consumer::{BaseConsumer, Consumer, DefaultConsumerContext};
use rdkafka::error::KafkaResult;
use rdkafka::{ClientConfig, Offset, TopicPartitionList};
use rlink::channel::utils::handover::Handover;
use rlink::core;
use rlink::core::checkpoint::{CheckpointFunction, CheckpointHandle, FunctionSnapshotContext};
use rlink::core::element::{FnSchema, Record};
use rlink::core::function::{Context, InputFormat, InputSplit, InputSplitSource, NamedFunction};
use rlink::core::properties::Properties;
use rlink::metrics::Tag;

use crate::source::checkpoint::KafkaCheckpointFunction;
use crate::source::consumer::{create_kafka_consumer, ConsumerRange};
use crate::source::deserializer::KafkaRecordDeserializerBuilder;
use crate::source::iterator::KafkaRecordIterator;
use crate::source::offset_range::{OffsetRange, PartitionOffset};
use crate::source::ConsumerRecord;

/// Depending on whether the task has `InputSplit`, and whether the client needs to be created
const CREATE_KAFKA_CONNECTION: &'static str = "create_kafka_connection";

pub struct KafkaInputFormat {
    name: String,
    parallelism: u16,

    client_config: ClientConfig,
    topics: Vec<String>,

    task_topic: String,
    task_partition: i32,

    buffer_size: usize,
    offset_range: OffsetRange,

    handover: Option<Handover<ConsumerRecord>>,

    deserializer_builder: Box<dyn KafkaRecordDeserializerBuilder>,
    schema: FnSchema,

    checkpoint: Option<KafkaCheckpointFunction>,
}

impl KafkaInputFormat {
    pub fn new(
        client_config: ClientConfig,
        topics: Vec<String>,
        buffer_size: usize,
        offset_range: OffsetRange,
        deserializer_builder: Box<dyn KafkaRecordDeserializerBuilder>,
        parallelism: u16,
        fn_name: String,
    ) -> Self {
        let schema = deserializer_builder.schema();
        KafkaInputFormat {
            name: fn_name,
            parallelism,
            client_config,
            topics,
            task_topic: "".to_string(),
            task_partition: 0,
            buffer_size,
            offset_range,
            handover: None,
            checkpoint: None,
            deserializer_builder,
            schema,
        }
    }

    fn consumer_ranges(&mut self, topic: String, partition: i32) -> KafkaResult<ConsumerRange> {
        let (begin_partition, end_partition) = match &self.offset_range {
            OffsetRange::None => {
                let state = self.checkpoint.as_mut().unwrap().as_state_mut();
                let begin_offset = state
                    .get()
                    .map(|offset| PartitionOffset { partition, offset });
                (begin_offset, None)
            }
            OffsetRange::Direct {
                begin_offset,
                end_offset,
            } => {
                let begin_partitions = {
                    let partition_offsets = begin_offset.get(topic.as_str()).unwrap();
                    partition_offsets.get(partition as usize).map(|p| p.clone())
                };
                let end_partitions = if let Some(end_offset) = end_offset {
                    let partition_offsets = end_offset.get(topic.as_str()).unwrap();
                    partition_offsets.get(partition as usize).map(|p| p.clone())
                } else {
                    None
                };

                (begin_partitions, end_partitions)
            }
            OffsetRange::Timestamp {
                begin_timestamp,
                end_timestamp,
            } => {
                let begin_timestamp = begin_timestamp.get(topic.as_str());
                let end_timestamp = end_timestamp
                    .as_ref()
                    .map(|x| x.get(topic.as_str()))
                    .unwrap_or_default();

                let consumer: BaseConsumer<DefaultConsumerContext> = self.client_config.create()?;

                fn offsets_for_times(
                    consumer: &BaseConsumer<DefaultConsumerContext>,
                    topic: &str,
                    partition: i32,
                    timestamp: u64,
                ) -> KafkaResult<Option<PartitionOffset>> {
                    let timeout = Duration::from_secs(3);
                    let mut partition_list = TopicPartitionList::with_capacity(1);
                    partition_list.set_partition_offset(
                        topic,
                        partition,
                        Offset::Offset(timestamp as i64),
                    )?;

                    let tpl = consumer.offsets_for_times(partition_list, timeout)?;
                    let partition_offset =
                        tpl.find_partition(topic, partition)
                            .map(|elem| PartitionOffset {
                                partition,
                                offset: elem.offset().to_raw().unwrap(),
                            });
                    Ok(partition_offset)
                }

                let begin_partition = match begin_timestamp {
                    Some(timestamp) => {
                        offsets_for_times(&consumer, topic.as_str(), partition, *timestamp)?
                    }
                    None => None,
                };
                let end_partition = match end_timestamp {
                    Some(timestamp) => {
                        offsets_for_times(&consumer, topic.as_str(), partition, *timestamp)?
                    }
                    None => None,
                };

                (begin_partition, end_partition)
            }
        };

        Ok(ConsumerRange {
            topic,
            partition,
            begin_offset: begin_partition
                .map(|x| x.offset)
                .unwrap_or(Offset::End.to_raw().unwrap()),
            end_offset: end_partition.map(|x| x.offset),
        })
    }
}

impl NamedFunction for KafkaInputFormat {
    fn name(&self) -> &str {
        self.name.as_str()
    }
}

impl InputFormat for KafkaInputFormat {
    fn open(&mut self, input_split: InputSplit, context: &Context) -> core::Result<()> {
        info!("kafka source open");

        self.task_topic = input_split.properties().get_string("topic").unwrap();
        self.task_partition = input_split.properties().get_i32("partition").unwrap();

        let kafka_checkpoint = KafkaCheckpointFunction::new(
            context.application_id.clone(),
            context.task_id,
            self.task_topic.as_str(),
            self.task_partition,
        );
        self.checkpoint = Some(kafka_checkpoint);

        self.initialize_state(&context.checkpoint_context(), &context.checkpoint_handle);

        let tags = vec![
            Tag::new("topic", self.task_topic.as_str()),
            Tag::new("partition", self.task_partition),
        ];
        self.handover = Some(Handover::<ConsumerRecord>::new(
            "KafkaSource_Handover",
            tags,
            self.buffer_size,
        ));

        let client_config = self.client_config.clone();
        let handover = self.handover.as_ref().unwrap().clone();

        let consumer_ranges = self
            .consumer_ranges(self.task_topic.to_string(), self.task_partition)
            .unwrap();
        create_kafka_consumer(
            context.task_id.job_id(),
            context.task_id.task_number(),
            client_config,
            consumer_ranges,
            handover,
            self.deserializer_builder.build(),
        );

        info!("start with consumer and operator mode");

        Ok(())
    }

    fn record_iter(&mut self) -> Box<dyn Iterator<Item = Record> + Send> {
        let handover = self.handover.as_ref().unwrap().clone();
        let state_recorder = self.checkpoint.as_mut().unwrap().as_state_mut().clone();
        Box::new(KafkaRecordIterator::new(handover, state_recorder))
    }

    fn close(&mut self) -> core::Result<()> {
        Ok(())
    }

    fn schema(&self, _input_schema: FnSchema) -> FnSchema {
        self.schema.clone()
    }

    fn parallelism(&self) -> u16 {
        self.parallelism
    }
}

impl CheckpointFunction for KafkaInputFormat {
    fn initialize_state(
        &mut self,
        context: &FunctionSnapshotContext,
        handle: &Option<CheckpointHandle>,
    ) {
        self.checkpoint
            .as_mut()
            .unwrap()
            .initialize_state(context, handle);
    }

    /// trigger the method when the `operator` operate a `Barrier` event
    fn snapshot_state(&mut self, context: &FunctionSnapshotContext) -> Option<CheckpointHandle> {
        match self.checkpoint.as_mut() {
            Some(checkpoint) => checkpoint.snapshot_state(context),
            None => None,
        }
    }
}

impl InputSplitSource for KafkaInputFormat {
    fn create_input_splits(&self, min_num_splits: u16) -> core::Result<Vec<InputSplit>> {
        let timeout = Duration::from_secs(3);

        info!("kafka config {:?}", self.client_config);

        let consumer: BaseConsumer = self
            .client_config
            .create()
            .map_err(|e| anyhow!("Consumer creation failed. {}", e))?;

        let mut input_splits = Vec::new();
        let mut index = 0;
        for topic in &self.topics {
            let metadata = consumer
                .fetch_metadata(Some(topic.as_str()), timeout)
                .map_err(|e| anyhow!("Failed to fetch metadata. {}", e))?;
            let metadata_topic = metadata
                .topics()
                .get(0)
                .ok_or(anyhow!("Topic({}) not found", topic))?;

            for partition in metadata_topic.partitions() {
                let mut properties = Properties::new();
                properties.set_str("topic", topic.as_str());
                properties.set_i32("partition", partition.id());
                properties.set_bool(CREATE_KAFKA_CONNECTION, true);

                let input_split = InputSplit::new(index, properties);
                index += 1;

                input_splits.push(input_split);
                if index == min_num_splits {
                    break;
                }
            }
        }

        if input_splits.len() > min_num_splits as usize {
            return Err(rlink::core::Error::from(
                "kafka `input_splits.len()` != `min_num_splits`",
            ));
        }

        if input_splits.len() < min_num_splits as usize {
            let mut extend_input_splits = Vec::new();
            let times = (min_num_splits as usize + input_splits.len() - 1) / input_splits.len();
            for _ in 1..times {
                for input_split in &input_splits {
                    let split_number = input_split.split_number();
                    let mut properties = input_split.properties().clone();
                    properties.set_bool(CREATE_KAFKA_CONNECTION, false);

                    extend_input_splits.push(InputSplit::new(split_number, properties));
                }
            }
            input_splits.extend_from_slice(extend_input_splits.as_slice());
        }

        Ok(input_splits)
    }
}
