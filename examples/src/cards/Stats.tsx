import React, { useState, useEffect } from "react";
import socialGraph from "../utils/socialGraph";

const Stats = () => {
  const [graphData, setGraphData] = useState({
    users: 0,
    follows: 0,
    sizeByDistance: {}
  });

  useEffect(() => {
    const updateGraphData = () => {
      setGraphData(socialGraph().size());
    };

    updateGraphData(); // Initial call
    const intervalId = setInterval(updateGraphData, 1000); // Update every second

    return () => clearInterval(intervalId); // Cleanup on unmount
  }, []);

  return (
    <div className="flex flex-col gap-4 p-4 rounded-xl bg-base-100">
      <div className="flex flex-row flex-wrap justify-between items-start gap-4 uppercase text-xs text-base-content/50 font-bold">
        Stored Social Graph Size
      </div>
      <div><strong>Users:</strong> {graphData.users}</div>
      <div><strong>Follow relationships:</strong> {graphData.follows}</div>
    </div>
  );
};

export default Stats;